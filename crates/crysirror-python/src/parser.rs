// SPDX-License-Identifier: MPL-2.0
//! Recursive-descent Python parser. The Python 3 grammar follows CPython
//! 3.12 (walrus, positional-only parameters, `match`, PEP 701 f-strings);
//! the Python 2 grammar adds `print`/`exec` statements, backticks, `<>`,
//! `except E, name`, tuple parameters and three-argument `raise`.

use crate::ast::*;
use crate::lexer::{tokenize, Tok, Token};
use crate::Dialect;
use num_bigint::BigInt;
use num_traits::Num;

#[derive(Clone, Debug)]
pub struct SyntaxError {
    pub msg: String,
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct ParseOptions {
    pub dialect: Dialect,
    /// Python 2 only: `print` is a statement (no `print_function` future).
    pub print_statement: bool,
}

impl ParseOptions {
    pub fn py3() -> Self {
        ParseOptions { dialect: Dialect::Py3, print_statement: false }
    }
    pub fn py2() -> Self {
        ParseOptions { dialect: Dialect::Py2, print_statement: false }
    }
}

pub fn parse_module(src: &str, opts: ParseOptions) -> Result<Vec<Stmt>, SyntaxError> {
    let mut p = Parser::new(src, opts)?;
    let mut body = Vec::new();
    while !p.at(&Tok::EndMarker) {
        if p.eat(&Tok::Newline) {
            continue;
        }
        p.statement(&mut body)?;
    }
    Ok(body)
}

pub fn parse_expression(src: &str, opts: ParseOptions) -> Result<Expr, SyntaxError> {
    let mut p = Parser::new(src, opts)?;
    p.depth = 1; // newlines are insignificant in eval input
    while p.eat(&Tok::Newline) {}
    let e = p.star_expressions()?;
    while p.eat(&Tok::Newline) {}
    if !p.at(&Tok::EndMarker) {
        return p.err("invalid syntax");
    }
    Ok(e)
}

/// Parses with the Python 2 fallback CPython/Ren'Py use: the `print_function`
/// grammar first, then the classic `print` statement grammar.
pub fn parse_module_py2_fallback(src: &str) -> Result<Vec<Stmt>, SyntaxError> {
    match parse_module(src, ParseOptions::py2()) {
        Ok(m) => Ok(m),
        Err(first) => parse_module(src, ParseOptions { dialect: Dialect::Py2, print_statement: true })
            .map_err(|_| first),
    }
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    opts: ParseOptions,
    depth: usize,
}

fn binop_of(op: &str) -> Option<BinOp> {
    Some(match op {
        "+" | "+=" => BinOp::Add,
        "-" | "-=" => BinOp::Sub,
        "*" | "*=" => BinOp::Mult,
        "@" | "@=" => BinOp::MatMult,
        "/" | "/=" => BinOp::Div,
        "%" | "%=" => BinOp::Mod,
        "**" | "**=" => BinOp::Pow,
        "<<" | "<<=" => BinOp::LShift,
        ">>" | ">>=" => BinOp::RShift,
        "|" | "|=" => BinOp::BitOr,
        "^" | "^=" => BinOp::BitXor,
        "&" | "&=" => BinOp::BitAnd,
        "//" | "//=" => BinOp::FloorDiv,
        _ => return None,
    })
}

const PY3_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in",
    "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
];
const PY2_KEYWORDS: &[&str] = &[
    "and", "as", "assert", "break", "class", "continue", "def", "del", "elif", "else", "except", "exec",
    "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "not", "or", "pass",
    "raise", "return", "try", "while", "with", "yield",
];

impl Parser {
    fn new(src: &str, opts: ParseOptions) -> Result<Parser, SyntaxError> {
        let toks = tokenize(src, opts.dialect).map_err(|e| SyntaxError { msg: e.msg, line: e.line, col: e.col })?;
        Ok(Parser { toks, pos: 0, opts, depth: 0 })
    }
    fn py2(&self) -> bool {
        self.opts.dialect == Dialect::Py2
    }
    fn tok(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    fn tok_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }
    fn loc(&self) -> Loc {
        let t = &self.toks[self.pos];
        Loc { line: t.line, col: t.col }
    }
    fn err<T>(&self, msg: impl Into<String>) -> Result<T, SyntaxError> {
        let t = &self.toks[self.pos];
        Err(SyntaxError { msg: msg.into(), line: t.line, col: t.col })
    }
    fn at(&self, t: &Tok) -> bool {
        self.tok() == t
    }
    fn at_op(&self, op: &str) -> bool {
        matches!(self.tok(), Tok::Op(o) if *o == op)
    }
    fn at_kw(&self, kw: &str) -> bool {
        matches!(self.tok(), Tok::Name(n) if n == kw) && self.is_keyword(kw)
    }
    fn is_keyword(&self, w: &str) -> bool {
        if self.py2() {
            PY2_KEYWORDS.contains(&w) || (w == "print" && self.opts.print_statement)
        } else {
            PY3_KEYWORDS.contains(&w)
        }
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.at(t) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn eat_op(&mut self, op: &str) -> bool {
        if self.at_op(op) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.at_kw(kw) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect_op(&mut self, op: &str) -> Result<(), SyntaxError> {
        if self.eat_op(op) {
            Ok(())
        } else {
            self.err(format!("expected '{op}'"))
        }
    }
    fn expect_kw(&mut self, kw: &str) -> Result<(), SyntaxError> {
        if self.eat_kw(kw) {
            Ok(())
        } else {
            self.err(format!("expected '{kw}'"))
        }
    }
    fn name(&mut self) -> Result<Ident, SyntaxError> {
        if let Tok::Name(n) = self.tok().clone() {
            if !self.is_keyword(&n) {
                self.pos += 1;
                return Ok(n);
            }
        }
        self.err("expected name")
    }
    fn expect_newline(&mut self) -> Result<(), SyntaxError> {
        if self.eat(&Tok::Newline) || self.at(&Tok::EndMarker) {
            Ok(())
        } else {
            self.err("invalid syntax")
        }
    }

    // ---- statements ----

    fn statement(&mut self, out: &mut Vec<Stmt>) -> Result<(), SyntaxError> {
        if let Some(s) = self.compound()? {
            out.push(s);
            return Ok(());
        }
        self.simple_stmts(out)
    }

    fn simple_stmts(&mut self, out: &mut Vec<Stmt>) -> Result<(), SyntaxError> {
        loop {
            out.push(self.simple_stmt()?);
            if !self.eat_op(";") {
                break;
            }
            if self.at(&Tok::Newline) || self.at(&Tok::EndMarker) {
                break;
            }
        }
        self.expect_newline()
    }

    fn block(&mut self) -> Result<Vec<Stmt>, SyntaxError> {
        self.expect_op(":")?;
        let mut body = Vec::new();
        if self.eat(&Tok::Newline) {
            while self.eat(&Tok::Newline) {}
            if !self.eat(&Tok::Indent) {
                return self.err("expected an indented block");
            }
            while !self.eat(&Tok::Dedent) {
                if self.at(&Tok::EndMarker) {
                    break;
                }
                if self.eat(&Tok::Newline) {
                    continue;
                }
                self.statement(&mut body)?;
            }
        } else {
            self.simple_stmts(&mut body)?;
        }
        Ok(body)
    }

    fn compound(&mut self) -> Result<Option<Stmt>, SyntaxError> {
        let loc = self.loc();
        let Tok::Name(w) = self.tok().clone() else {
            if self.at_op("@") {
                return self.decorated().map(Some);
            }
            return Ok(None);
        };
        let kind = match w.as_str() {
            "if" if self.is_keyword("if") => {
                self.pos += 1;
                return self.if_stmt(loc).map(Some);
            }
            "while" if self.is_keyword("while") => {
                self.pos += 1;
                let test = self.named_expression()?;
                let body = self.block()?;
                let orelse = if self.eat_kw("else") { self.block()? } else { Vec::new() };
                StmtKind::While { test, body, orelse }
            }
            "for" if self.is_keyword("for") => {
                self.pos += 1;
                return self.for_stmt(loc, false).map(Some);
            }
            "try" if self.is_keyword("try") => {
                self.pos += 1;
                return self.try_stmt(loc).map(Some);
            }
            "with" if self.is_keyword("with") => {
                self.pos += 1;
                return self.with_stmt(loc, false).map(Some);
            }
            "def" if self.is_keyword("def") => {
                self.pos += 1;
                return self.funcdef(loc, Vec::new(), false).map(Some);
            }
            "class" if self.is_keyword("class") => {
                self.pos += 1;
                return self.classdef(loc, Vec::new()).map(Some);
            }
            "async" if !self.py2() => {
                self.pos += 1;
                if self.eat_kw("def") {
                    return self.funcdef(loc, Vec::new(), true).map(Some);
                }
                if self.eat_kw("for") {
                    return self.for_stmt(loc, true).map(Some);
                }
                if self.eat_kw("with") {
                    return self.with_stmt(loc, true).map(Some);
                }
                return self.err("invalid syntax");
            }
            "match" if !self.py2() && self.looks_like_match() => {
                self.pos += 1;
                return self.match_stmt(loc).map(Some);
            }
            _ => return Ok(None),
        };
        Ok(Some(Stmt { loc, kind }))
    }

    /// `type` is a soft keyword (3.12): `type Name = ...` or
    /// `type Name[T, ...] = ...`.
    fn looks_like_type_alias(&self) -> bool {
        matches!(self.tok(), Tok::Name(w) if w == "type")
            && matches!(self.tok_at(1), Tok::Name(n) if !self.is_keyword(n))
            && matches!(self.tok_at(2), Tok::Op("=" | "["))
    }

    /// `type Name[T, *Ts, **P] = value`, desugared to
    /// `Name = (lambda T, Ts, P: typing.TypeAliasType("Name", lambda: value,
    /// type_params=(T, Ts, P)))(TypeVar("T"), TypeVarTuple("Ts"), ParamSpec("P"))`
    /// so the value is evaluated lazily with the parameters in scope.
    fn type_alias(&mut self, loc: Loc) -> Result<Stmt, SyntaxError> {
        let name = self.name()?;
        let mut params: Vec<(String, &'static str)> = Vec::new();
        if self.eat_op("[") {
            loop {
                let kind = if self.eat_op("**") {
                    "ParamSpec"
                } else if self.eat_op("*") {
                    "TypeVarTuple"
                } else {
                    "TypeVar"
                };
                params.push((self.name()?, kind));
                if self.eat_op(":") {
                    self.test()?;
                }
                if self.eat_op("=") {
                    self.test()?;
                }
                if !self.eat_op(",") || matches!(self.tok(), Tok::Op("]")) {
                    break;
                }
            }
            self.expect_op("]")?;
        }
        self.expect_op("=")?;
        let value = self.test()?;
        let e = |kind: ExprKind| Expr::new(loc.clone(), kind);
        let load = |id: &str| Expr::new(loc.clone(), ExprKind::Name { id: id.to_owned(), ctx: Ctx::Load });
        let s = |v: &str| Expr::new(loc.clone(), ExprKind::Constant(Constant::Str(v.to_owned())));
        let typing = |attr: &str| {
            let imp = Expr::new(
                loc.clone(),
                ExprKind::Call { func: Box::new(load("__import__")), args: vec![s("typing")], keywords: Vec::new() },
            );
            Expr::new(loc.clone(), ExprKind::Attribute { value: Box::new(imp), attr: attr.to_owned(), ctx: Ctx::Load })
        };
        let lazy = e(ExprKind::Lambda { args: Box::new(Arguments::default()), body: Box::new(value) });
        let tparams = e(ExprKind::Tuple { elts: params.iter().map(|(n, _)| load(n)).collect(), ctx: Ctx::Load });
        let alias = e(ExprKind::Call {
            func: Box::new(typing("TypeAliasType")),
            args: vec![s(&name), lazy],
            keywords: vec![Keyword { arg: Some("type_params".into()), value: tparams }],
        });
        let mut outer = Arguments::default();
        for (n, _) in &params {
            outer.args.push(Arg { name: n.clone(), annotation: None, unpack: None });
        }
        let maker = e(ExprKind::Lambda { args: Box::new(outer), body: Box::new(alias) });
        let args = params
            .iter()
            .map(|(n, kind)| e(ExprKind::Call { func: Box::new(typing(kind)), args: vec![s(n)], keywords: Vec::new() }))
            .collect();
        let value = e(ExprKind::Call { func: Box::new(maker), args, keywords: Vec::new() });
        let target = Expr::new(loc.clone(), ExprKind::Name { id: name, ctx: Ctx::Store });
        Ok(Stmt { loc, kind: StmtKind::Assign { targets: vec![target], value } })
    }

    fn looks_like_match(&self) -> bool {
        // `match` is a soft keyword: a statement only when followed by an
        // expression and a ':' that ends the logical line.
        if matches!(self.tok_at(1), Tok::Op("=") | Tok::Op(".") | Tok::Op("(") | Tok::Newline | Tok::Op(":") | Tok::Op(","))
        {
            if !matches!(self.tok_at(1), Tok::Op("(")) {
                return false;
            }
        }
        let mut depth = 0i32;
        let mut i = self.pos + 1;
        while i < self.toks.len() {
            match &self.toks[i].tok {
                Tok::Op("(" | "[" | "{") => depth += 1,
                Tok::Op(")" | "]" | "}") => depth -= 1,
                Tok::Op(":") if depth == 0 => {
                    return matches!(self.toks.get(i + 1).map(|t| &t.tok), Some(Tok::Newline));
                }
                Tok::Op("=") if depth == 0 => return false,
                Tok::Newline | Tok::EndMarker => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn decorated(&mut self) -> Result<Stmt, SyntaxError> {
        let mut decorators = Vec::new();
        while self.eat_op("@") {
            decorators.push(self.named_expression()?);
            self.expect_newline()?;
        }
        let loc = self.loc();
        if self.eat_kw("def") {
            return self.funcdef(loc, decorators, false);
        }
        if self.eat_kw("class") {
            return self.classdef(loc, decorators);
        }
        if !self.py2() && matches!(self.tok(), Tok::Name(n) if n == "async") {
            self.pos += 1;
            self.expect_kw("def")?;
            return self.funcdef(loc, decorators, true);
        }
        self.err("expected function or class after decorator")
    }

    fn if_stmt(&mut self, loc: Loc) -> Result<Stmt, SyntaxError> {
        let test = self.named_expression()?;
        let body = self.block()?;
        let orelse = if self.at_kw("elif") {
            let l = self.loc();
            self.pos += 1;
            vec![self.if_stmt(l)?]
        } else if self.eat_kw("else") {
            self.block()?
        } else {
            Vec::new()
        };
        Ok(Stmt { loc, kind: StmtKind::If { test, body, orelse } })
    }

    fn for_stmt(&mut self, loc: Loc, is_async: bool) -> Result<Stmt, SyntaxError> {
        let target = self.target_list()?;
        self.expect_kw("in")?;
        let iter = self.star_expressions()?;
        let body = self.block()?;
        let orelse = if self.eat_kw("else") { self.block()? } else { Vec::new() };
        Ok(Stmt { loc, kind: StmtKind::For { target, iter, body, orelse, is_async } })
    }

    fn target_list(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        let mut elts = vec![self.star_target()?];
        let mut tuple = false;
        while self.eat_op(",") {
            tuple = true;
            if self.at_kw("in") || self.at_op("=") || self.at_op(")") {
                break;
            }
            elts.push(self.star_target()?);
        }
        if tuple {
            Ok(Expr::new(loc, ExprKind::Tuple { elts, ctx: Ctx::Store }))
        } else {
            Ok(elts.pop().unwrap())
        }
    }

    fn star_target(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        if self.eat_op("*") {
            let v = self.bitor()?.with_ctx(Ctx::Store);
            return Ok(Expr::new(loc, ExprKind::Starred { value: Box::new(v), ctx: Ctx::Store }));
        }
        Ok(self.bitor()?.with_ctx(Ctx::Store))
    }

    fn try_stmt(&mut self, loc: Loc) -> Result<Stmt, SyntaxError> {
        let body = self.block()?;
        let mut handlers = Vec::new();
        while self.at_kw("except") {
            let hloc = self.loc();
            self.pos += 1;
            let _star = !self.py2() && self.eat_op("*");
            let mut typ = None;
            let mut name = None;
            if !self.at_op(":") {
                typ = Some(self.test()?);
                if self.at_op(",") && !self.py2() {
                    // `except A, B:` is a Python 3 syntax error; parenthesised
                    // tuples go through test().
                    return self.err("multiple exception types must be parenthesized");
                }
                if self.eat_kw("as") {
                    let l = self.loc();
                    name = Some(Expr::new(l, ExprKind::Name { id: self.name()?, ctx: Ctx::Store }));
                } else if self.py2() && self.eat_op(",") {
                    name = Some(self.test()?.with_ctx(Ctx::Store));
                }
            }
            let hbody = self.block()?;
            handlers.push(Handler { loc: hloc, typ, name, body: hbody });
        }
        let orelse = if self.eat_kw("else") { self.block()? } else { Vec::new() };
        let finalbody = if self.eat_kw("finally") { self.block()? } else { Vec::new() };
        if handlers.is_empty() && finalbody.is_empty() {
            return self.err("expected 'except' or 'finally' block");
        }
        Ok(Stmt { loc, kind: StmtKind::Try { body, handlers, orelse, finalbody } })
    }

    fn with_stmt(&mut self, loc: Loc, is_async: bool) -> Result<Stmt, SyntaxError> {
        let mut items = Vec::new();
        // Parenthesised with-items (3.10) are detected by a ')' followed by ':'.
        let paren = self.at_op("(") && self.paren_with_items();
        if paren {
            self.pos += 1;
        }
        loop {
            let context = self.test()?;
            let vars = if self.eat_kw("as") { Some(self.star_target()?) } else { None };
            items.push(WithItem { context, vars });
            if !self.eat_op(",") {
                break;
            }
            if paren && self.at_op(")") {
                break;
            }
        }
        if paren {
            self.expect_op(")")?;
        }
        let body = self.block()?;
        Ok(Stmt { loc, kind: StmtKind::With { items, body, is_async } })
    }

    fn paren_with_items(&self) -> bool {
        let mut depth = 0i32;
        let mut i = self.pos;
        let mut saw_as = false;
        while i < self.toks.len() {
            match &self.toks[i].tok {
                Tok::Op("(" | "[" | "{") => depth += 1,
                Tok::Op(")" | "]" | "}") => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(self.toks.get(i + 1).map(|t| &t.tok), Some(Tok::Op(":"))) && saw_as;
                    }
                }
                Tok::Name(n) if n == "as" && depth == 1 => saw_as = true,
                Tok::Newline | Tok::EndMarker => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn funcdef(&mut self, loc: Loc, decorators: Vec<Expr>, is_async: bool) -> Result<Stmt, SyntaxError> {
        let name = self.name()?;
        self.expect_op("(")?;
        let args = self.parameters(")", true)?;
        self.expect_op(")")?;
        let returns = if self.eat_op("->") { Some(Box::new(self.test()?)) } else { None };
        let body = self.block()?;
        Ok(Stmt { loc, kind: StmtKind::FunctionDef { name, args: Box::new(args), body, decorators, returns, is_async } })
    }

    fn classdef(&mut self, loc: Loc, decorators: Vec<Expr>) -> Result<Stmt, SyntaxError> {
        let name = self.name()?;
        let (mut bases, mut keywords) = (Vec::new(), Vec::new());
        if self.eat_op("(") {
            let (a, k) = self.call_args()?;
            bases = a;
            keywords = k;
        }
        let body = self.block()?;
        Ok(Stmt { loc, kind: StmtKind::ClassDef { name, bases, keywords, body, decorators } })
    }

    /// Parameter list for `def` (annotations allowed) or `lambda`.
    fn parameters(&mut self, close: &str, annotations: bool) -> Result<Arguments, SyntaxError> {
        let mut a = Arguments::default();
        let mut seen_default = false;
        let mut kwonly = false;
        loop {
            if self.at_op(close) {
                break;
            }
            if self.eat_op("**") {
                a.kwarg = Some(self.param(annotations)?);
                self.eat_op(",");
                break;
            }
            if self.eat_op("*") {
                kwonly = true;
                if !self.at_op(",") && !self.at_op(close) {
                    a.vararg = Some(self.param(annotations)?);
                }
            } else if self.eat_op("/") {
                a.posonly.append(&mut a.args);
            } else {
                let p = if self.py2() && self.at_op("(") {
                    // Tuple parameter.
                    let l = self.loc();
                    self.pos += 1;
                    let t = self.py2_tuple_param()?;
                    self.expect_op(")")?;
                    let name = format!(".{}", a.args.len() + a.posonly.len());
                    let _ = l;
                    Arg { name, annotation: None, unpack: Some(Box::new(t)) }
                } else {
                    self.param(annotations)?
                };
                if self.eat_op("=") {
                    let d = self.test()?;
                    if kwonly {
                        a.kw_defaults.push(Some(d));
                    } else {
                        seen_default = true;
                        a.defaults.push(d);
                    }
                } else if kwonly {
                    a.kw_defaults.push(None);
                } else if seen_default {
                    return self.err("non-default argument follows default argument");
                }
                if kwonly {
                    a.kwonly.push(p);
                } else {
                    a.args.push(p);
                }
            }
            if !self.eat_op(",") {
                break;
            }
        }
        Ok(a)
    }

    fn py2_tuple_param(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        let mut elts = Vec::new();
        loop {
            if self.eat_op("(") {
                elts.push(self.py2_tuple_param()?);
                self.expect_op(")")?;
            } else {
                let l = self.loc();
                elts.push(Expr::new(l, ExprKind::Name { id: self.name()?, ctx: Ctx::Store }));
            }
            if !self.eat_op(",") || self.at_op(")") {
                break;
            }
        }
        Ok(Expr::new(loc, ExprKind::Tuple { elts, ctx: Ctx::Store }))
    }

    fn param(&mut self, annotations: bool) -> Result<Arg, SyntaxError> {
        let name = self.name()?;
        let annotation = if annotations && self.eat_op(":") { Some(Box::new(self.test()?)) } else { None };
        Ok(Arg { name, annotation, unpack: None })
    }

    fn match_stmt(&mut self, loc: Loc) -> Result<Stmt, SyntaxError> {
        let subject = self.star_named_expressions_tuple()?;
        self.expect_op(":")?;
        self.expect_newline()?;
        while self.eat(&Tok::Newline) {}
        if !self.eat(&Tok::Indent) {
            return self.err("expected an indented block");
        }
        let mut cases = Vec::new();
        while !self.eat(&Tok::Dedent) {
            if self.eat(&Tok::Newline) {
                continue;
            }
            if !matches!(self.tok(), Tok::Name(n) if n == "case") {
                return self.err("expected 'case'");
            }
            self.pos += 1;
            let pattern = self.patterns()?;
            let guard = if self.eat_kw("if") { Some(self.named_expression()?) } else { None };
            let body = self.block()?;
            cases.push(MatchCase { pattern, guard, body });
        }
        Ok(Stmt { loc, kind: StmtKind::Match { subject, cases } })
    }

    fn patterns(&mut self) -> Result<Pattern, SyntaxError> {
        let first = self.as_pattern()?;
        if self.at_op(",") {
            let mut v = vec![first];
            while self.eat_op(",") {
                if self.at_op(":") || self.at_kw("if") {
                    break;
                }
                v.push(self.as_pattern()?);
            }
            return Ok(Pattern::Sequence(v));
        }
        Ok(first)
    }

    fn as_pattern(&mut self) -> Result<Pattern, SyntaxError> {
        let p = self.or_pattern()?;
        if self.eat_kw("as") {
            let name = self.name()?;
            return Ok(Pattern::As { pattern: Some(Box::new(p)), name: Some(name) });
        }
        Ok(p)
    }

    fn or_pattern(&mut self) -> Result<Pattern, SyntaxError> {
        let mut v = vec![self.closed_pattern()?];
        while self.eat_op("|") {
            v.push(self.closed_pattern()?);
        }
        Ok(if v.len() == 1 { v.pop().unwrap() } else { Pattern::Or(v) })
    }

    fn closed_pattern(&mut self) -> Result<Pattern, SyntaxError> {
        let loc = self.loc();
        if self.eat_op("*") {
            let n = self.name()?;
            return Ok(Pattern::Star(if n == "_" { None } else { Some(n) }));
        }
        if self.eat_op("(") || self.at_op("[") {
            let close = if self.eat_op("[") { "]" } else { ")" };
            let mut v = Vec::new();
            let mut comma = false;
            while !self.at_op(close) {
                v.push(self.as_pattern()?);
                if !self.eat_op(",") {
                    break;
                }
                comma = true;
            }
            self.expect_op(close)?;
            if close == ")" && v.len() == 1 && !comma {
                return Ok(v.pop().unwrap());
            }
            return Ok(Pattern::Sequence(v));
        }
        if self.eat_op("{") {
            let (mut keys, mut patterns, mut rest) = (Vec::new(), Vec::new(), None);
            while !self.at_op("}") {
                if self.eat_op("**") {
                    rest = Some(self.name()?);
                } else {
                    keys.push(self.pattern_value()?);
                    self.expect_op(":")?;
                    patterns.push(self.as_pattern()?);
                }
                if !self.eat_op(",") {
                    break;
                }
            }
            self.expect_op("}")?;
            return Ok(Pattern::Mapping { keys, patterns, rest });
        }
        match self.tok().clone() {
            Tok::Name(n) if n == "None" || n == "True" || n == "False" => {
                self.pos += 1;
                return Ok(Pattern::Singleton(match n.as_str() {
                    "None" => Constant::None,
                    "True" => Constant::Bool(true),
                    _ => Constant::Bool(false),
                }));
            }
            Tok::Name(n) if !self.is_keyword(&n) => {
                self.pos += 1;
                let mut e = Expr::new(loc.clone(), ExprKind::Name { id: n.clone(), ctx: Ctx::Load });
                let mut dotted = false;
                while self.eat_op(".") {
                    dotted = true;
                    let attr = self.name()?;
                    e = Expr::new(loc.clone(), ExprKind::Attribute { value: Box::new(e), attr, ctx: Ctx::Load });
                }
                if self.eat_op("(") {
                    let (mut patterns, mut kwd_attrs, mut kwd_patterns) = (Vec::new(), Vec::new(), Vec::new());
                    while !self.at_op(")") {
                        if matches!(self.tok(), Tok::Name(_)) && matches!(self.tok_at(1), Tok::Op("=")) {
                            kwd_attrs.push(self.name()?);
                            self.pos += 1;
                            kwd_patterns.push(self.as_pattern()?);
                        } else {
                            patterns.push(self.as_pattern()?);
                        }
                        if !self.eat_op(",") {
                            break;
                        }
                    }
                    self.expect_op(")")?;
                    return Ok(Pattern::Class { cls: e, patterns, kwd_attrs, kwd_patterns });
                }
                if dotted {
                    return Ok(Pattern::Value(e));
                }
                return Ok(Pattern::As { pattern: None, name: if n == "_" { None } else { Some(n) } });
            }
            _ => {}
        }
        Ok(Pattern::Value(self.pattern_value()?))
    }

    fn pattern_value(&mut self) -> Result<Expr, SyntaxError> {
        // Literal patterns: numbers (with sign / complex), strings, dotted names.
        self.arith()
    }

    fn simple_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let loc = self.loc();
        if !self.py2() && self.looks_like_type_alias() {
            self.pos += 1;
            return self.type_alias(loc);
        }
        if let Tok::Name(w) = self.tok().clone() {
            if self.is_keyword(&w) || (w == "print" && self.opts.print_statement) {
                let kind = match w.as_str() {
                    "pass" => {
                        self.pos += 1;
                        StmtKind::Pass
                    }
                    "break" => {
                        self.pos += 1;
                        StmtKind::Break
                    }
                    "continue" => {
                        self.pos += 1;
                        StmtKind::Continue
                    }
                    "return" => {
                        self.pos += 1;
                        let v = if self.end_of_simple() { None } else { Some(self.star_expressions()?) };
                        StmtKind::Return(v)
                    }
                    "raise" => {
                        self.pos += 1;
                        self.raise_stmt()?
                    }
                    "global" => {
                        self.pos += 1;
                        StmtKind::Global(self.name_list()?)
                    }
                    "nonlocal" => {
                        self.pos += 1;
                        StmtKind::Nonlocal(self.name_list()?)
                    }
                    "del" => {
                        self.pos += 1;
                        let mut v = Vec::new();
                        loop {
                            v.push(self.bitor()?.with_ctx(Ctx::Del));
                            if !self.eat_op(",") || self.end_of_simple() {
                                break;
                            }
                        }
                        StmtKind::Delete(v)
                    }
                    "assert" => {
                        self.pos += 1;
                        let test = self.test()?;
                        let msg = if self.eat_op(",") { Some(self.test()?) } else { None };
                        StmtKind::Assert { test, msg }
                    }
                    "import" => {
                        self.pos += 1;
                        self.import_stmt()?
                    }
                    "from" => {
                        self.pos += 1;
                        self.from_import()?
                    }
                    "print" => {
                        self.pos += 1;
                        self.print_stmt()?
                    }
                    "exec" if self.py2() => {
                        self.pos += 1;
                        self.exec_stmt()?
                    }
                    _ => return self.expr_stmt(loc),
                };
                return Ok(Stmt { loc, kind });
            }
        }
        self.expr_stmt(loc)
    }

    fn end_of_simple(&self) -> bool {
        matches!(self.tok(), Tok::Newline | Tok::EndMarker | Tok::Op(";"))
    }

    fn name_list(&mut self) -> Result<Vec<Ident>, SyntaxError> {
        let mut v = vec![self.name()?];
        while self.eat_op(",") {
            v.push(self.name()?);
        }
        Ok(v)
    }

    fn raise_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        if self.end_of_simple() {
            return Ok(StmtKind::Raise { exc: None, cause: None, py2_value: None, py2_tb: None });
        }
        let exc = Some(self.test()?);
        if self.py2() && self.eat_op(",") {
            let v = Some(self.test()?);
            let tb = if self.eat_op(",") { Some(self.test()?) } else { None };
            return Ok(StmtKind::Raise { exc, cause: None, py2_value: v, py2_tb: tb });
        }
        let cause = if self.eat_kw("from") { Some(self.test()?) } else { None };
        Ok(StmtKind::Raise { exc, cause, py2_value: None, py2_tb: None })
    }

    fn dotted_name(&mut self) -> Result<String, SyntaxError> {
        let mut s = self.name()?;
        while self.eat_op(".") {
            s.push('.');
            s.push_str(&self.name()?);
        }
        Ok(s)
    }

    fn import_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        let mut v = Vec::new();
        loop {
            let name = self.dotted_name()?;
            let asname = if self.eat_kw("as") { Some(self.name()?) } else { None };
            v.push(Alias { name, asname });
            if !self.eat_op(",") {
                break;
            }
        }
        Ok(StmtKind::Import(v))
    }

    fn from_import(&mut self) -> Result<StmtKind, SyntaxError> {
        let mut level = 0;
        loop {
            if self.eat_op(".") {
                level += 1;
            } else if self.eat_op("...") {
                level += 3;
            } else {
                break;
            }
        }
        let module = if self.at_kw("import") { None } else { Some(self.dotted_name()?) };
        self.expect_kw("import")?;
        let mut names = Vec::new();
        if self.eat_op("*") {
            names.push(Alias { name: "*".into(), asname: None });
        } else {
            let paren = self.eat_op("(");
            loop {
                if paren && self.at_op(")") {
                    break;
                }
                let name = self.name()?;
                let asname = if self.eat_kw("as") { Some(self.name()?) } else { None };
                names.push(Alias { name, asname });
                if !self.eat_op(",") {
                    break;
                }
            }
            if paren {
                self.expect_op(")")?;
            }
        }
        Ok(StmtKind::ImportFrom { module, names, level })
    }

    fn print_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        let mut dest = None;
        let mut values = Vec::new();
        let mut nl = true;
        if self.eat_op(">>") {
            dest = Some(self.test()?);
            if !self.eat_op(",") {
                return Ok(StmtKind::Print { dest, values, nl });
            }
        }
        while !self.end_of_simple() {
            values.push(self.test()?);
            if self.eat_op(",") {
                nl = false;
            } else {
                nl = true;
                break;
            }
        }
        if values.is_empty() {
            nl = true;
        }
        Ok(StmtKind::Print { dest, values, nl })
    }

    fn exec_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        let body = self.bitor()?;
        let (mut globals, mut locals) = (None, None);
        if self.eat_kw("in") {
            globals = Some(self.test()?);
            if self.eat_op(",") {
                locals = Some(self.test()?);
            }
        } else if let ExprKind::Tuple { elts, .. } = &body.kind {
            // `exec(code, globals[, locals])` compatibility form.
            if (2..=3).contains(&elts.len()) {
                let mut it = elts.clone().into_iter();
                let b = it.next().unwrap();
                globals = it.next();
                locals = it.next();
                return Ok(StmtKind::Exec { body: b, globals, locals });
            }
        }
        Ok(StmtKind::Exec { body, globals, locals })
    }

    fn expr_stmt(&mut self, loc: Loc) -> Result<Stmt, SyntaxError> {
        let first = self.star_expressions_or_yield()?;
        if self.eat_op(":") {
            let simple = matches!(first.kind, ExprKind::Name { .. });
            let annotation = self.test()?;
            let value = if self.eat_op("=") { Some(self.star_expressions_or_yield()?) } else { None };
            return Ok(Stmt {
                loc,
                kind: StmtKind::AnnAssign { target: first.with_ctx(Ctx::Store), annotation, value, simple },
            });
        }
        if let Tok::Op(op) = self.tok().clone() {
            if op.ends_with('=') && !matches!(op, "==" | "<=" | ">=" | "!=" | "=") {
                if let Some(bop) = binop_of(op) {
                    self.pos += 1;
                    let value = self.star_expressions_or_yield()?;
                    return Ok(Stmt { loc, kind: StmtKind::AugAssign { target: first.with_ctx(Ctx::Store), op: bop, value } });
                }
            }
        }
        if self.at_op("=") {
            let mut targets = vec![first];
            let mut value;
            loop {
                self.expect_op("=")?;
                value = self.star_expressions_or_yield()?;
                if !self.at_op("=") {
                    break;
                }
                targets.push(value);
            }
            let targets = targets.into_iter().map(|t| t.with_ctx(Ctx::Store)).collect();
            return Ok(Stmt { loc, kind: StmtKind::Assign { targets, value } });
        }
        Ok(Stmt { loc, kind: StmtKind::Expr(first) })
    }

    // ---- expressions ----

    fn star_expressions_or_yield(&mut self) -> Result<Expr, SyntaxError> {
        if self.at_kw("yield") {
            return self.yield_expr();
        }
        self.star_expressions()
    }

    fn yield_expr(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        self.expect_kw("yield")?;
        if self.eat_kw("from") {
            let e = self.test()?;
            return Ok(Expr::new(loc, ExprKind::YieldFrom(Box::new(e))));
        }
        if self.end_of_simple() || self.at_op(")") || self.at_op("]") || self.at_op("}") || self.at_op("=") {
            return Ok(Expr::new(loc, ExprKind::Yield(None)));
        }
        let e = self.star_expressions()?;
        Ok(Expr::new(loc, ExprKind::Yield(Some(Box::new(e)))))
    }

    /// Comma-separated expressions, forming a tuple when a comma appears.
    fn star_expressions(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        let first = self.star_expression()?;
        if !self.at_op(",") {
            return Ok(first);
        }
        let mut elts = vec![first];
        while self.eat_op(",") {
            if self.expr_terminates() {
                break;
            }
            elts.push(self.star_expression()?);
        }
        Ok(Expr::new(loc, ExprKind::Tuple { elts, ctx: Ctx::Load }))
    }

    fn star_named_expressions_tuple(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        let first = self.star_named_expression()?;
        if !self.at_op(",") {
            return Ok(first);
        }
        let mut elts = vec![first];
        while self.eat_op(",") {
            if self.expr_terminates() {
                break;
            }
            elts.push(self.star_named_expression()?);
        }
        Ok(Expr::new(loc, ExprKind::Tuple { elts, ctx: Ctx::Load }))
    }

    fn expr_terminates(&self) -> bool {
        match self.tok() {
            Tok::Newline | Tok::EndMarker => true,
            Tok::Op(o) => matches!(*o, ")" | "]" | "}" | "=" | ";" | ":") || (o.ends_with('=') && o.len() > 1 && !matches!(*o, "==" | "<=" | ">=" | "!=")),
            Tok::Name(n) => (n == "in" || n == "for" || n == "if" || n == "async") && self.is_keyword(n) || n == "async",
            _ => false,
        }
    }

    fn star_expression(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        if self.eat_op("*") {
            let v = self.bitor()?;
            return Ok(Expr::new(loc, ExprKind::Starred { value: Box::new(v), ctx: Ctx::Load }));
        }
        self.test()
    }

    fn star_named_expression(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        if self.eat_op("*") {
            let v = self.bitor()?;
            return Ok(Expr::new(loc, ExprKind::Starred { value: Box::new(v), ctx: Ctx::Load }));
        }
        self.named_expression()
    }

    fn named_expression(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        if !self.py2() && matches!(self.tok(), Tok::Name(_)) && matches!(self.tok_at(1), Tok::Op(":=")) {
            let id = self.name()?;
            self.pos += 1;
            let value = self.test()?;
            return Ok(Expr::new(
                loc.clone(),
                ExprKind::NamedExpr {
                    target: Box::new(Expr::new(loc, ExprKind::Name { id, ctx: Ctx::Store })),
                    value: Box::new(value),
                },
            ));
        }
        self.test()
    }

    /// `test` in CPython's grammar: conditional expression or lambda.
    fn test(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        if self.at_kw("lambda") {
            return self.lambda(true);
        }
        let body = self.or_test()?;
        if self.at_kw("if") {
            // Disambiguate from a comprehension `if` clause: a conditional
            // expression must have an `else`.
            let save = self.pos;
            self.pos += 1;
            let test = self.or_test()?;
            if self.eat_kw("else") {
                let orelse = self.test()?;
                return Ok(Expr::new(
                    loc,
                    ExprKind::IfExp { test: Box::new(test), body: Box::new(body), orelse: Box::new(orelse) },
                ));
            }
            self.pos = save;
        }
        Ok(body)
    }

    fn test_nocond(&mut self) -> Result<Expr, SyntaxError> {
        if self.at_kw("lambda") {
            return self.lambda(false);
        }
        self.or_test()
    }

    fn lambda(&mut self, allow_cond: bool) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        self.expect_kw("lambda")?;
        let args = self.parameters(":", false)?;
        self.expect_op(":")?;
        let body = if allow_cond { self.test()? } else { self.test_nocond()? };
        Ok(Expr::new(loc, ExprKind::Lambda { args: Box::new(args), body: Box::new(body) }))
    }

    fn or_test(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        let first = self.and_test()?;
        if !self.at_kw("or") {
            return Ok(first);
        }
        let mut values = vec![first];
        while self.eat_kw("or") {
            values.push(self.and_test()?);
        }
        Ok(Expr::new(loc, ExprKind::BoolOp { op: BoolOp::Or, values }))
    }

    fn and_test(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        let first = self.not_test()?;
        if !self.at_kw("and") {
            return Ok(first);
        }
        let mut values = vec![first];
        while self.eat_kw("and") {
            values.push(self.not_test()?);
        }
        Ok(Expr::new(loc, ExprKind::BoolOp { op: BoolOp::And, values }))
    }

    fn not_test(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        if self.eat_kw("not") {
            let e = self.not_test()?;
            return Ok(Expr::new(loc, ExprKind::UnaryOp { op: UnaryOp::Not, operand: Box::new(e) }));
        }
        self.comparison()
    }

    fn comp_op(&mut self) -> Option<CmpOp> {
        let op = match self.tok() {
            Tok::Op("<") => CmpOp::Lt,
            Tok::Op(">") => CmpOp::Gt,
            Tok::Op("==") => CmpOp::Eq,
            Tok::Op(">=") => CmpOp::GtE,
            Tok::Op("<=") => CmpOp::LtE,
            Tok::Op("!=") | Tok::Op("<>") => CmpOp::NotEq,
            Tok::Name(n) if n == "in" => CmpOp::In,
            Tok::Name(n) if n == "is" => {
                if matches!(self.tok_at(1), Tok::Name(m) if m == "not") {
                    self.pos += 2;
                    return Some(CmpOp::IsNot);
                }
                CmpOp::Is
            }
            Tok::Name(n) if n == "not" && matches!(self.tok_at(1), Tok::Name(m) if m == "in") => {
                self.pos += 2;
                return Some(CmpOp::NotIn);
            }
            _ => return None,
        };
        self.pos += 1;
        Some(op)
    }

    fn comparison(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        let left = self.bitor()?;
        let mut ops = Vec::new();
        let mut comparators = Vec::new();
        while let Some(op) = self.comp_op() {
            ops.push(op);
            comparators.push(self.bitor()?);
        }
        if ops.is_empty() {
            return Ok(left);
        }
        Ok(Expr::new(loc, ExprKind::Compare { left: Box::new(left), ops, comparators }))
    }

    fn binary(&mut self, ops: &[&str], next: fn(&mut Parser) -> Result<Expr, SyntaxError>) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        let mut left = next(self)?;
        loop {
            let Tok::Op(o) = self.tok().clone() else { break };
            if !ops.contains(&o) {
                break;
            }
            self.pos += 1;
            let right = next(self)?;
            left = Expr::new(loc.clone(), ExprKind::BinOp { left: Box::new(left), op: binop_of(o).unwrap(), right: Box::new(right) });
        }
        Ok(left)
    }

    fn bitor(&mut self) -> Result<Expr, SyntaxError> {
        self.binary(&["|"], Parser::bitxor)
    }
    fn bitxor(&mut self) -> Result<Expr, SyntaxError> {
        self.binary(&["^"], Parser::bitand)
    }
    fn bitand(&mut self) -> Result<Expr, SyntaxError> {
        self.binary(&["&"], Parser::shift)
    }
    fn shift(&mut self) -> Result<Expr, SyntaxError> {
        self.binary(&["<<", ">>"], Parser::arith)
    }
    fn arith(&mut self) -> Result<Expr, SyntaxError> {
        self.binary(&["+", "-"], Parser::term)
    }
    fn term(&mut self) -> Result<Expr, SyntaxError> {
        self.binary(&["*", "/", "%", "//", "@"], Parser::factor)
    }

    fn factor(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        let op = match self.tok() {
            Tok::Op("+") => UnaryOp::UAdd,
            Tok::Op("-") => UnaryOp::USub,
            Tok::Op("~") => UnaryOp::Invert,
            _ => return self.power(),
        };
        self.pos += 1;
        let e = self.factor()?;
        Ok(Expr::new(loc, ExprKind::UnaryOp { op, operand: Box::new(e) }))
    }

    fn power(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        let base = if !self.py2() && matches!(self.tok(), Tok::Name(n) if n == "await") {
            self.pos += 1;
            let e = self.primary()?;
            Expr::new(loc.clone(), ExprKind::Await(Box::new(e)))
        } else {
            self.primary()?
        };
        if self.eat_op("**") {
            let exp = self.factor()?;
            return Ok(Expr::new(loc, ExprKind::BinOp { left: Box::new(base), op: BinOp::Pow, right: Box::new(exp) }));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<Expr, SyntaxError> {
        let mut e = self.atom()?;
        loop {
            let loc = self.loc();
            if self.eat_op(".") {
                let attr = match self.tok().clone() {
                    // Attribute names may be keywords in neither dialect, but
                    // Python 2 allows `print`/`exec` as attributes.
                    Tok::Name(n) if !self.is_keyword(&n) || n == "print" || n == "exec" => {
                        self.pos += 1;
                        n
                    }
                    _ => return self.err("expected attribute name"),
                };
                e = Expr::new(loc, ExprKind::Attribute { value: Box::new(e), attr, ctx: Ctx::Load });
            } else if self.eat_op("(") {
                let (args, keywords) = self.call_args()?;
                e = Expr::new(loc, ExprKind::Call { func: Box::new(e), args, keywords });
            } else if self.eat_op("[") {
                let s = self.subscript_list()?;
                self.expect_op("]")?;
                e = Expr::new(loc, ExprKind::Subscript { value: Box::new(e), slice: Box::new(s), ctx: Ctx::Load });
            } else {
                break;
            }
        }
        Ok(e)
    }

    /// Parses arguments up to and including ')'.
    fn call_args(&mut self) -> Result<(Vec<Expr>, Vec<Keyword>), SyntaxError> {
        let mut args = Vec::new();
        let mut kws = Vec::new();
        while !self.at_op(")") {
            let loc = self.loc();
            if self.eat_op("**") {
                kws.push(Keyword { arg: None, value: self.test()? });
            } else if self.eat_op("*") {
                let v = self.test()?;
                args.push(Expr::new(loc, ExprKind::Starred { value: Box::new(v), ctx: Ctx::Load }));
            } else if matches!(self.tok(), Tok::Name(_)) && matches!(self.tok_at(1), Tok::Op("=")) {
                let n = match self.tok().clone() {
                    Tok::Name(n) => n,
                    _ => unreachable!(),
                };
                self.pos += 2;
                kws.push(Keyword { arg: Some(n), value: self.test()? });
            } else {
                let e = self.named_expression()?;
                if self.at_kw("for") || (self.at_kw("async") && !self.py2()) {
                    let generators = self.comp_for(false)?;
                    args.push(Expr::new(loc, ExprKind::Comp { kind: CompKind::Gen, elt: Box::new(e), generators }));
                } else {
                    args.push(e);
                }
            }
            if !self.eat_op(",") {
                break;
            }
        }
        self.expect_op(")")?;
        Ok((args, kws))
    }

    fn subscript_list(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        let first = self.subscript()?;
        if !self.at_op(",") {
            return Ok(first);
        }
        let mut elts = vec![first];
        while self.eat_op(",") {
            if self.at_op("]") {
                break;
            }
            elts.push(self.subscript()?);
        }
        Ok(Expr::new(loc, ExprKind::Tuple { elts, ctx: Ctx::Load }))
    }

    fn subscript(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        if self.at_op("*") {
            return self.star_expression();
        }
        let lower = if self.at_op(":") { None } else { Some(Box::new(self.named_expression()?)) };
        if !self.eat_op(":") {
            return Ok(*lower.unwrap());
        }
        let upper = if self.at_op(":") || self.at_op("]") || self.at_op(",") { None } else { Some(Box::new(self.test()?)) };
        let step = if self.eat_op(":") {
            if self.at_op("]") || self.at_op(",") { None } else { Some(Box::new(self.test()?)) }
        } else {
            None
        };
        Ok(Expr::new(loc, ExprKind::Slice { lower, upper, step }))
    }

    fn comp_for(&mut self, py2_list: bool) -> Result<Vec<Comprehension>, SyntaxError> {
        let mut gens = Vec::new();
        loop {
            if !self.py2() && matches!(self.tok(), Tok::Name(n) if n == "async") {
                self.pos += 1;
            }
            if !self.eat_kw("for") {
                break;
            }
            let target = self.target_list()?;
            self.expect_kw("in")?;
            let iter = if py2_list {
                let first = self.test_nocond()?;
                if self.eat_op(",") {
                    // Python 2 list_for uses testlist_safe. Generator, set
                    // and dictionary comprehensions use the scalar grammar.
                    let loc = first.loc.clone();
                    let mut elts = vec![first, self.test_nocond()?];
                    while self.eat_op(",") {
                        if self.at_op("]") || self.at_kw("if") || self.at_kw("for") {
                            break;
                        }
                        elts.push(self.test_nocond()?);
                    }
                    Expr::new(loc, ExprKind::Tuple { elts, ctx: Ctx::Load })
                } else {
                    first
                }
            } else {
                self.or_test()?
            };
            let mut ifs = Vec::new();
            while self.eat_kw("if") {
                ifs.push(self.test_nocond()?);
            }
            gens.push(Comprehension { target, iter, ifs });
        }
        Ok(gens)
    }

    fn atom(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        match self.tok().clone() {
            Tok::Name(n) => {
                if self.is_keyword(&n) {
                    match n.as_str() {
                        "None" => {
                            self.pos += 1;
                            return Ok(Expr::new(loc, ExprKind::Constant(Constant::None)));
                        }
                        "True" => {
                            self.pos += 1;
                            return Ok(Expr::new(loc, ExprKind::Constant(Constant::Bool(true))));
                        }
                        "False" => {
                            self.pos += 1;
                            return Ok(Expr::new(loc, ExprKind::Constant(Constant::Bool(false))));
                        }
                        _ => return self.err("invalid syntax"),
                    }
                }
                self.pos += 1;
                Ok(Expr::new(loc, ExprKind::Name { id: n, ctx: Ctx::Load }))
            }
            Tok::Int { digits, radix, .. } => {
                self.pos += 1;
                let v = BigInt::from_str_radix(&digits, radix).map_err(|_| SyntaxError { msg: "invalid integer".into(), line: loc.line, col: loc.col })?;
                Ok(Expr::new(loc, ExprKind::Constant(Constant::Int(v))))
            }
            Tok::Float(f) => {
                self.pos += 1;
                Ok(Expr::new(loc, ExprKind::Constant(Constant::Float(f))))
            }
            Tok::Imag(f) => {
                self.pos += 1;
                Ok(Expr::new(loc, ExprKind::Constant(Constant::Complex(0.0, f))))
            }
            Tok::Str { .. } => self.strings(),
            Tok::Op("...") => {
                self.pos += 1;
                Ok(Expr::new(loc, ExprKind::Constant(Constant::Ellipsis)))
            }
            Tok::Op("`") if self.py2() => {
                self.pos += 1;
                let e = self.star_expressions()?;
                self.expect_op("`")?;
                Ok(Expr::new(loc, ExprKind::Repr(Box::new(e))))
            }
            Tok::Op("(") => {
                self.pos += 1;
                self.depth += 1;
                if self.eat_op(")") {
                    self.depth -= 1;
                    return Ok(Expr::new(loc, ExprKind::Tuple { elts: Vec::new(), ctx: Ctx::Load }));
                }
                if self.at_kw("yield") {
                    let y = self.yield_expr()?;
                    self.expect_op(")")?;
                    self.depth -= 1;
                    return Ok(y);
                }
                let first = self.star_named_expression()?;
                if self.at_kw("for") || (!self.py2() && matches!(self.tok(), Tok::Name(n) if n == "async")) {
                    let generators = self.comp_for(false)?;
                    self.expect_op(")")?;
                    self.depth -= 1;
                    return Ok(Expr::new(loc, ExprKind::Comp { kind: CompKind::Gen, elt: Box::new(first), generators }));
                }
                if self.eat_op(")") {
                    self.depth -= 1;
                    if matches!(first.kind, ExprKind::Starred { .. }) {
                        return self.err("cannot use starred expression here");
                    }
                    return Ok(first);
                }
                let mut elts = vec![first];
                while self.eat_op(",") {
                    if self.at_op(")") {
                        break;
                    }
                    elts.push(self.star_named_expression()?);
                }
                self.expect_op(")")?;
                self.depth -= 1;
                Ok(Expr::new(loc, ExprKind::Tuple { elts, ctx: Ctx::Load }))
            }
            Tok::Op("[") => {
                self.pos += 1;
                if self.eat_op("]") {
                    return Ok(Expr::new(loc, ExprKind::List { elts: Vec::new(), ctx: Ctx::Load }));
                }
                let first = self.star_named_expression()?;
                if self.at_kw("for") || (!self.py2() && matches!(self.tok(), Tok::Name(n) if n == "async")) {
                    let generators = self.comp_for(self.py2())?;
                    self.expect_op("]")?;
                    return Ok(Expr::new(loc, ExprKind::Comp { kind: CompKind::List, elt: Box::new(first), generators }));
                }
                let mut elts = vec![first];
                while self.eat_op(",") {
                    if self.at_op("]") {
                        break;
                    }
                    elts.push(self.star_named_expression()?);
                }
                self.expect_op("]")?;
                Ok(Expr::new(loc, ExprKind::List { elts, ctx: Ctx::Load }))
            }
            Tok::Op("{") => {
                self.pos += 1;
                self.dict_or_set(loc)
            }
            _ => self.err("invalid syntax"),
        }
    }

    fn dict_or_set(&mut self, loc: Loc) -> Result<Expr, SyntaxError> {
        if self.eat_op("}") {
            return Ok(Expr::new(loc, ExprKind::Dict { keys: Vec::new(), values: Vec::new() }));
        }
        if self.eat_op("**") {
            let v = self.bitor()?;
            return self.dict_rest(loc, vec![None], vec![v]);
        }
        let first = self.star_named_expression()?;
        if self.eat_op(":") {
            let value = self.test()?;
            if self.at_kw("for") || (!self.py2() && matches!(self.tok(), Tok::Name(n) if n == "async")) {
                let generators = self.comp_for(false)?;
                self.expect_op("}")?;
                return Ok(Expr::new(loc, ExprKind::DictComp { key: Box::new(first), value: Box::new(value), generators }));
            }
            return self.dict_rest(loc, vec![Some(first)], vec![value]);
        }
        if self.at_kw("for") || (!self.py2() && matches!(self.tok(), Tok::Name(n) if n == "async")) {
            let generators = self.comp_for(false)?;
            self.expect_op("}")?;
            return Ok(Expr::new(loc, ExprKind::Comp { kind: CompKind::Set, elt: Box::new(first), generators }));
        }
        let mut elts = vec![first];
        while self.eat_op(",") {
            if self.at_op("}") {
                break;
            }
            elts.push(self.star_named_expression()?);
        }
        self.expect_op("}")?;
        Ok(Expr::new(loc, ExprKind::Set(elts)))
    }

    fn dict_rest(&mut self, loc: Loc, mut keys: Vec<Option<Expr>>, mut values: Vec<Expr>) -> Result<Expr, SyntaxError> {
        while self.eat_op(",") {
            if self.at_op("}") {
                break;
            }
            if self.eat_op("**") {
                keys.push(None);
                values.push(self.bitor()?);
            } else {
                keys.push(Some(self.test()?));
                self.expect_op(":")?;
                values.push(self.test()?);
            }
        }
        self.expect_op("}")?;
        Ok(Expr::new(loc, ExprKind::Dict { keys, values }))
    }

    fn strings(&mut self) -> Result<Expr, SyntaxError> {
        let loc = self.loc();
        let mut parts: Vec<FPart> = Vec::new();
        let mut bytes: Option<Vec<u8>> = None;
        let mut any_f = false;
        let mut any_text = false;
        while let Tok::Str { prefix, body, .. } = self.tok().clone() {
            let tloc = self.loc();
            self.pos += 1;
            let raw = prefix.contains('r');
            if prefix.contains('b') {
                if any_text {
                    return self.err("cannot mix bytes and nonbytes literals");
                }
                let b = decode_bytes(&body, raw).map_err(|m| SyntaxError { msg: m, line: tloc.line, col: tloc.col })?;
                bytes.get_or_insert_with(Vec::new).extend(b);
                continue;
            }
            if bytes.is_some() {
                return self.err("cannot mix bytes and nonbytes literals");
            }
            any_text = true;
            if prefix.contains('f') {
                any_f = true;
                let mut fp = self.fstring(&body, raw, &tloc)?;
                parts.append(&mut fp);
            } else {
                let s = decode_str(&body, raw, self.py2() && !prefix.contains('u') && raw)
                    .map_err(|m| SyntaxError { msg: m, line: tloc.line, col: tloc.col })?;
                push_lit(&mut parts, s);
            }
        }
        if let Some(b) = bytes {
            return Ok(Expr::new(loc, ExprKind::Constant(Constant::Bytes(b))));
        }
        if !any_f {
            let s = match parts.pop() {
                Some(FPart::Lit(s)) => s,
                _ => String::new(),
            };
            return Ok(Expr::new(loc, ExprKind::Constant(Constant::Str(s))));
        }
        Ok(Expr::new(loc, ExprKind::FString(parts)))
    }

    fn fstring(&self, body: &str, raw: bool, loc: &Loc) -> Result<Vec<FPart>, SyntaxError> {
        let e = |m: &str| SyntaxError { msg: format!("f-string: {m}"), line: loc.line, col: loc.col };
        let chars: Vec<char> = body.chars().collect();
        let mut parts = Vec::new();
        let mut lit = String::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '{' {
                if chars.get(i + 1) == Some(&'{') {
                    lit.push('{');
                    i += 2;
                    continue;
                }
                if !lit.is_empty() {
                    let s = decode_str(&core::mem::take(&mut lit), raw, false).map_err(|m| e(&m))?;
                    push_lit(&mut parts, s);
                }
                let (field, next) = self.fstring_field(&chars, i + 1, raw, loc)?;
                parts.push(field);
                i = next;
                continue;
            }
            if c == '}' {
                if chars.get(i + 1) == Some(&'}') {
                    lit.push('}');
                    i += 2;
                    continue;
                }
                return Err(e("single '}' is not allowed"));
            }
            lit.push(c);
            i += 1;
        }
        if !lit.is_empty() {
            let s = decode_str(&lit, raw, false).map_err(|m| e(&m))?;
            push_lit(&mut parts, s);
        }
        Ok(parts)
    }

    /// Parses one `{expr!c:spec}` starting just after '{'. Returns the index
    /// after the closing '}'.
    fn fstring_field(&self, chars: &[char], start: usize, raw: bool, loc: &Loc) -> Result<(FPart, usize), SyntaxError> {
        let e = |m: &str| SyntaxError { msg: format!("f-string: {m}"), line: loc.line, col: loc.col };
        let mut depth = 0i32;
        let mut i = start;
        let mut quote: Option<char> = None;
        // Find the end of the expression: '!' (not '!='), ':' or '}' at depth 0.
        let expr_end;
        loop {
            let Some(&c) = chars.get(i) else { return Err(e("expecting '}'")) };
            if let Some(q) = quote {
                if c == '\\' {
                    i += 2;
                    continue;
                }
                if c == q {
                    quote = None;
                }
                i += 1;
                continue;
            }
            match c {
                '\'' | '"' => quote = Some(c),
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' if depth > 0 => depth -= 1,
                '}' | ':' if depth == 0 => {
                    expr_end = i;
                    break;
                }
                '!' if depth == 0 && chars.get(i + 1) != Some(&'=') => {
                    expr_end = i;
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        let mut expr_text: String = chars[start..expr_end].iter().collect();
        let mut debug_text = None;
        let trimmed = expr_text.trim_end();
        if trimmed.ends_with('=') && !trimmed.ends_with("==") && !trimmed.ends_with("!=") && !trimmed.ends_with("<=") && !trimmed.ends_with(">=") {
            debug_text = Some(expr_text.clone());
            let cut = trimmed.len() - 1;
            expr_text.truncate(cut);
        }
        let value = parse_expression(&format!("({expr_text})"), self.opts).map_err(|se| e(&se.msg))?;
        let mut conversion = None;
        if chars.get(i) == Some(&'!') {
            conversion = chars.get(i + 1).copied();
            if !matches!(conversion, Some('r' | 's' | 'a')) {
                return Err(e("invalid conversion character"));
            }
            i += 2;
        }
        let mut spec = None;
        if chars.get(i) == Some(&':') {
            i += 1;
            let mut sp = Vec::new();
            let mut lit = String::new();
            loop {
                let Some(&c) = chars.get(i) else { return Err(e("expecting '}'")) };
                if c == '}' {
                    break;
                }
                if c == '{' {
                    if !lit.is_empty() {
                        push_lit(&mut sp, decode_str(&core::mem::take(&mut lit), raw, false).map_err(|m| e(&m))?);
                    }
                    let (f, n) = self.fstring_field(chars, i + 1, raw, loc)?;
                    sp.push(f);
                    i = n;
                    continue;
                }
                lit.push(c);
                i += 1;
            }
            if !lit.is_empty() {
                push_lit(&mut sp, decode_str(&lit, raw, false).map_err(|m| e(&m))?);
            }
            spec = Some(sp);
        }
        if chars.get(i) != Some(&'}') {
            return Err(e("expecting '}'"));
        }
        if debug_text.is_some() && conversion.is_none() && spec.is_none() {
            conversion = Some('r');
        }
        Ok((FPart::Field { value: Box::new(value), conversion, spec, debug_text }, i + 1))
    }
}

fn push_lit(parts: &mut Vec<FPart>, s: String) {
    if let Some(FPart::Lit(prev)) = parts.last_mut() {
        prev.push_str(&s);
    } else {
        parts.push(FPart::Lit(s));
    }
}

/// Decodes escape sequences of a text literal. `py2_raw_unicode_only`
/// selects Python 2 `ur''` behaviour, where only `\u`/`\U` are processed.
pub fn decode_str(body: &str, raw: bool, py2_raw_unicode_only: bool) -> Result<String, String> {
    if raw && !py2_raw_unicode_only {
        return Ok(body.to_string());
    }
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '\\' || i + 1 >= chars.len() {
            out.push(c);
            i += 1;
            continue;
        }
        let n = chars[i + 1];
        if py2_raw_unicode_only && n != 'u' && n != 'U' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 2;
        match n {
            '\n' => {}
            '\\' => out.push('\\'),
            '\'' => out.push('\''),
            '"' => out.push('"'),
            'a' => out.push('\x07'),
            'b' => out.push('\x08'),
            'f' => out.push('\x0c'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'v' => out.push('\x0b'),
            '0'..='7' => {
                let mut v = n.to_digit(8).unwrap();
                for _ in 0..2 {
                    if let Some(d) = chars.get(i).and_then(|c| c.to_digit(8)) {
                        v = v * 8 + d;
                        i += 1;
                    } else {
                        break;
                    }
                }
                out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
            }
            'x' | 'u' | 'U' => {
                let len = match n {
                    'x' => 2,
                    'u' => 4,
                    _ => 8,
                };
                let hex: String = chars.get(i..i + len).ok_or("truncated escape")?.iter().collect();
                let v = u32::from_str_radix(&hex, 16).map_err(|_| "truncated escape".to_string())?;
                i += len;
                out.push(char::from_u32(v).ok_or("illegal Unicode character")?);
            }
            'N' => {
                if chars.get(i) != Some(&'{') {
                    return Err("malformed \\N character escape".into());
                }
                let end = chars[i..].iter().position(|c| *c == '}').ok_or("malformed \\N character escape")? + i;
                let name: String = chars[i + 1..end].iter().collect();
                out.push(crate::unicode_names::lookup(&name).ok_or_else(|| format!("unknown Unicode character name {name}"))?);
                i = end + 1;
            }
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    Ok(out)
}

pub fn decode_bytes(body: &str, raw: bool) -> Result<Vec<u8>, String> {
    if raw {
        return body.chars().map(|c| u8::try_from(c as u32).map_err(|_| "bytes can only contain ASCII literal characters".to_string())).collect();
    }
    let chars: Vec<char> = body.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '\\' || i + 1 >= chars.len() {
            out.push(u8::try_from(c as u32).map_err(|_| "bytes can only contain ASCII literal characters".to_string())?);
            i += 1;
            continue;
        }
        let n = chars[i + 1];
        i += 2;
        match n {
            '\n' => {}
            '\\' => out.push(b'\\'),
            '\'' => out.push(b'\''),
            '"' => out.push(b'"'),
            'a' => out.push(7),
            'b' => out.push(8),
            'f' => out.push(12),
            'n' => out.push(b'\n'),
            'r' => out.push(b'\r'),
            't' => out.push(b'\t'),
            'v' => out.push(11),
            '0'..='7' => {
                let mut v = n.to_digit(8).unwrap();
                for _ in 0..2 {
                    if let Some(d) = chars.get(i).and_then(|c| c.to_digit(8)) {
                        v = v * 8 + d;
                        i += 1;
                    } else {
                        break;
                    }
                }
                out.push(v as u8);
            }
            'x' => {
                let hex: String = chars.get(i..i + 2).ok_or("truncated \\x escape")?.iter().collect();
                out.push(u8::from_str_radix(&hex, 16).map_err(|_| "invalid \\x escape".to_string())?);
                i += 2;
            }
            other => {
                out.push(b'\\');
                out.push(other as u8);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_common_forms() {
        let src = "@dec\ndef f(a, /, b=1, *args, c, d=2, **kw) -> int:\n    x = [i*2 for i in range(3) if i]\n    return {k: v for k, v in kw.items()}\nclass C(B, metaclass=M):\n    pass\ny = lambda x, *a: x if a else -x\nz = f'{y!r:>{w}} {a=}'\nwith open(p) as f, g:\n    pass\nmatch v:\n    case [1, *rest] | (2, 3):\n        pass\n    case Point(x=0) as p if p:\n        pass\n";
        let m = parse_module(src, ParseOptions::py3()).unwrap();
        assert_eq!(m.len(), 6);
    }
    #[test]
    fn py2_statements() {
        let src = "print >>f, 'a', b,\nexec code in g\ntry:\n    pass\nexcept E, e:\n    raise E, 'x'\nx = `y` <> 1\ndef f((a, b), c):\n    pass\n";
        let m = parse_module(src, ParseOptions { dialect: Dialect::Py2, print_statement: true }).unwrap();
        assert!(matches!(m[0].kind, StmtKind::Print { nl: false, .. }));
        assert!(matches!(m[1].kind, StmtKind::Exec { .. }));
        assert!(parse_module_py2_fallback("print 'hello'\n").is_ok());
    }
}
