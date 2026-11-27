// SPDX-License-Identifier: MPL-2.0
//! `_sre`: a Python-syntax regular expression engine. Patterns compile to a
//! backtracking program executed with an explicit stack (no host recursion
//! per character), supporting groups, named groups, backreferences,
//! lookaround, atomic groups, possessive/lazy quantifiers and inline flags.

use super::{module_with, set};
use crate::builtins::{parse, Kw};
use crate::dict::Dict;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};
use std::rc::Rc;

pub const I: u32 = 2;
pub const L: u32 = 4;
pub const M: u32 = 8;
pub const S: u32 = 16;
pub const U: u32 = 32;
pub const X: u32 = 64;
pub const A: u32 = 256;

#[derive(Clone, Debug)]
enum ClassItem {
    Ch(char),
    Range(char, char),
    Digit(bool),
    Word(bool),
    Space(bool),
}

#[derive(Clone, Debug)]
enum Node {
    Empty,
    Char(char),
    Any,
    Class(Vec<ClassItem>, bool),
    Bol,
    Eol,
    StrStart,
    StrEnd,
    WordB(bool),
    Group(Box<Node>, Option<usize>),
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Repeat { node: Box<Node>, min: u32, max: Option<u32>, greedy: bool, possessive: bool },
    Backref(usize),
    Look { node: Box<Node>, ahead: bool, neg: bool },
    Atomic(Box<Node>),
    Cond { group: usize, yes: Box<Node>, no: Box<Node> },
    /// Scoped flags `(?i:...)`.
    Flags(Box<Node>, u32, u32),
}

struct ReParser<'a> {
    c: &'a [char],
    p: usize,
    flags: u32,
    ngroups: usize,
    names: Vec<(String, usize)>,
}

type PErr = String;

impl<'a> ReParser<'a> {
    fn peek(&self) -> Option<char> {
        self.c.get(self.p).copied()
    }
    fn eat(&mut self, ch: char) -> bool {
        if self.peek() == Some(ch) {
            self.p += 1;
            true
        } else {
            false
        }
    }
    fn skip_verbose(&mut self) {
        if self.flags & X == 0 {
            return;
        }
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => self.p += 1,
                Some('#') => {
                    while let Some(c) = self.peek() {
                        self.p += 1;
                        if c == '\n' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }
    fn alt(&mut self) -> Result<Node, PErr> {
        let mut alts = vec![self.concat()?];
        while self.eat('|') {
            alts.push(self.concat()?);
        }
        Ok(if alts.len() == 1 { alts.pop().unwrap() } else { Node::Alt(alts) })
    }
    fn concat(&mut self) -> Result<Node, PErr> {
        let mut items = Vec::new();
        loop {
            self.skip_verbose();
            match self.peek() {
                None | Some('|') | Some(')') => break,
                _ => {}
            }
            let atom = self.atom()?;
            let atom = self.quant(atom)?;
            items.push(atom);
        }
        Ok(if items.len() == 1 { items.pop().unwrap() } else if items.is_empty() { Node::Empty } else { Node::Concat(items) })
    }
    fn number(&mut self) -> Option<u32> {
        let st = self.p;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.p += 1;
        }
        if self.p == st {
            return None;
        }
        self.c[st..self.p].iter().collect::<String>().parse().ok()
    }
    fn quant(&mut self, atom: Node) -> Result<Node, PErr> {
        self.skip_verbose();
        let (min, max) = match self.peek() {
            Some('*') => {
                self.p += 1;
                (0, None)
            }
            Some('+') => {
                self.p += 1;
                (1, None)
            }
            Some('?') => {
                self.p += 1;
                (0, Some(1))
            }
            Some('{') => {
                let save = self.p;
                self.p += 1;
                let lo = self.number();
                let (min, max) = if self.eat(',') {
                    let hi = self.number();
                    (lo.unwrap_or(0), hi)
                } else {
                    match lo {
                        Some(n) => (n, Some(n)),
                        None => {
                            self.p = save;
                            return Ok(atom);
                        }
                    }
                };
                if !self.eat('}') {
                    self.p = save;
                    return Ok(atom);
                }
                if let Some(mx) = max {
                    if mx < min {
                        return Err("min repeat greater than max repeat".into());
                    }
                }
                (min, max)
            }
            _ => return Ok(atom),
        };
        if matches!(atom, Node::Empty | Node::Bol | Node::Eol | Node::StrStart | Node::StrEnd | Node::WordB(_)) {
            return Err("nothing to repeat".into());
        }
        let mut greedy = true;
        let mut possessive = false;
        if self.eat('?') {
            greedy = false;
        } else if self.eat('+') {
            possessive = true;
        }
        if matches!(self.peek(), Some('*') | Some('+')) && !possessive {
            return Err("multiple repeat".into());
        }
        Ok(Node::Repeat { node: Box::new(atom), min, max, greedy, possessive })
    }
    fn escape_class(&mut self, c: char) -> Result<ClassItem, PErr> {
        Ok(match c {
            'd' => ClassItem::Digit(false),
            'D' => ClassItem::Digit(true),
            'w' => ClassItem::Word(false),
            'W' => ClassItem::Word(true),
            's' => ClassItem::Space(false),
            'S' => ClassItem::Space(true),
            _ => ClassItem::Ch(self.escape_char(c)?),
        })
    }
    fn escape_char(&mut self, c: char) -> Result<char, PErr> {
        Ok(match c {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            'f' => '\x0c',
            'v' => '\x0b',
            'a' => '\x07',
            '0' => {
                let mut v = 0u32;
                let mut n = 0;
                while n < 2 && self.peek().is_some_and(|d| d.is_digit(8)) {
                    v = v * 8 + self.peek().unwrap().to_digit(8).unwrap();
                    self.p += 1;
                    n += 1;
                }
                char::from_u32(v).unwrap_or('\0')
            }
            'x' | 'u' | 'U' => {
                let n = match c {
                    'x' => 2,
                    'u' => 4,
                    _ => 8,
                };
                let h: String = self.c.get(self.p..self.p + n).ok_or("incomplete escape")?.iter().collect();
                self.p += n;
                char::from_u32(u32::from_str_radix(&h, 16).map_err(|_| "bad escape")?).ok_or("bad escape")?
            }
            'N' => {
                if !self.eat('{') {
                    return Err("missing {".into());
                }
                let st = self.p;
                while self.peek().is_some_and(|c| c != '}') {
                    self.p += 1;
                }
                let name: String = self.c[st..self.p].iter().collect();
                self.p += 1;
                crate::unicode_names::lookup(&name).ok_or("undefined character name")?
            }
            c if c.is_ascii_alphanumeric() => return Err(format!("bad escape \\{c}")),
            c => c,
        })
    }
    fn class(&mut self) -> Result<Node, PErr> {
        let neg = self.eat('^');
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let Some(c) = self.peek() else { return Err("unterminated character set".into()) };
            if c == ']' && !first {
                self.p += 1;
                break;
            }
            first = false;
            self.p += 1;
            let item = if c == '\\' {
                let e = self.peek().ok_or("bad escape (end of pattern)")?;
                self.p += 1;
                if e == 'b' { ClassItem::Ch('\x08') } else { self.escape_class(e)? }
            } else if c == '[' && self.peek() == Some(':') {
                // POSIX classes are literal in Python.
                ClassItem::Ch('[')
            } else {
                ClassItem::Ch(c)
            };
            if let ClassItem::Ch(lo) = item {
                if self.peek() == Some('-') && self.c.get(self.p + 1).is_some_and(|n| *n != ']') {
                    self.p += 1;
                    let hc = self.peek().unwrap();
                    self.p += 1;
                    let hi = if hc == '\\' {
                        let e = self.peek().ok_or("bad escape")?;
                        self.p += 1;
                        match self.escape_class(e)? {
                            ClassItem::Ch(x) => x,
                            _ => return Err("bad character range".into()),
                        }
                    } else {
                        hc
                    };
                    if hi < lo {
                        return Err(format!("bad character range {lo}-{hi}"));
                    }
                    items.push(ClassItem::Range(lo, hi));
                    continue;
                }
            }
            items.push(item);
        }
        Ok(Node::Class(items, neg))
    }
    fn group_name(&mut self, end: char) -> Result<String, PErr> {
        let st = self.p;
        while self.peek().is_some_and(|c| c != end) {
            self.p += 1;
        }
        let n: String = self.c[st..self.p].iter().collect();
        if !self.eat(end) {
            return Err("missing group name terminator".into());
        }
        Ok(n)
    }
    fn atom(&mut self) -> Result<Node, PErr> {
        let c = self.peek().unwrap();
        self.p += 1;
        Ok(match c {
            '.' => Node::Any,
            '^' => Node::Bol,
            '$' => Node::Eol,
            '[' => self.class()?,
            '(' => {
                if self.eat('?') {
                    match self.peek() {
                        Some(':') => {
                            self.p += 1;
                            let n = self.alt()?;
                            self.expect_close()?;
                            Node::Group(Box::new(n), None)
                        }
                        Some('P') => {
                            self.p += 1;
                            if self.eat('<') {
                                let name = self.group_name('>')?;
                                self.ngroups += 1;
                                let idx = self.ngroups;
                                self.names.push((name, idx));
                                let n = self.alt()?;
                                self.expect_close()?;
                                Node::Group(Box::new(n), Some(idx))
                            } else if self.eat('=') {
                                let name = self.group_name(')')?;
                                let idx = self.names.iter().find(|(n, _)| *n == name).map(|(_, i)| *i).ok_or(format!("unknown group name '{name}'"))?;
                                Node::Backref(idx)
                            } else {
                                return Err("unknown extension ?P".into());
                            }
                        }
                        Some('<') if matches!(self.c.get(self.p + 1), Some('=') | Some('!')) => {
                            self.p += 1;
                            let neg = self.peek() == Some('!');
                            self.p += 1;
                            let n = self.alt()?;
                            self.expect_close()?;
                            Node::Look { node: Box::new(n), ahead: false, neg }
                        }
                        Some('<') => {
                            self.p += 1;
                            let name = self.group_name('>')?;
                            self.ngroups += 1;
                            let idx = self.ngroups;
                            self.names.push((name, idx));
                            let n = self.alt()?;
                            self.expect_close()?;
                            Node::Group(Box::new(n), Some(idx))
                        }
                        Some('=') | Some('!') => {
                            let neg = self.peek() == Some('!');
                            self.p += 1;
                            let n = self.alt()?;
                            self.expect_close()?;
                            Node::Look { node: Box::new(n), ahead: true, neg }
                        }
                        Some('>') => {
                            self.p += 1;
                            let n = self.alt()?;
                            self.expect_close()?;
                            Node::Atomic(Box::new(n))
                        }
                        Some('#') => {
                            while self.peek().is_some_and(|c| c != ')') {
                                self.p += 1;
                            }
                            self.expect_close()?;
                            Node::Empty
                        }
                        Some('(') => {
                            self.p += 1;
                            let name = self.group_name(')')?;
                            let group = match name.parse::<usize>() {
                                Ok(n) => n,
                                Err(_) => self.names.iter().find(|(n, _)| *n == name).map(|(_, i)| *i).ok_or("unknown group name")?,
                            };
                            let yes = self.concat()?;
                            let no = if self.eat('|') { self.concat()? } else { Node::Empty };
                            self.expect_close()?;
                            Node::Cond { group, yes: Box::new(yes), no: Box::new(no) }
                        }
                        _ => {
                            // Inline flags: (?aiLmsux) or (?flags-flags:...)
                            let mut on = 0u32;
                            let mut off = 0u32;
                            let mut neg = false;
                            loop {
                                let Some(f) = self.peek() else { return Err("missing -, : or )".into()) };
                                self.p += 1;
                                let bit = match f {
                                    'i' => I,
                                    'm' => M,
                                    's' => S,
                                    'x' => X,
                                    'a' => A,
                                    'u' => U,
                                    'L' => L,
                                    '-' => {
                                        neg = true;
                                        continue;
                                    }
                                    ':' => {
                                        let saved = self.flags;
                                        self.flags = (self.flags | on) & !off;
                                        let n = self.alt()?;
                                        self.flags = saved;
                                        self.expect_close()?;
                                        return Ok(Node::Flags(Box::new(n), on, off));
                                    }
                                    ')' => {
                                        self.flags |= on;
                                        return Ok(Node::Flags(Box::new(Node::Empty), on, 0).global_marker());
                                    }
                                    _ => return Err(format!("unknown flag '{f}'")),
                                };
                                if neg {
                                    off |= bit;
                                } else {
                                    on |= bit;
                                }
                            }
                        }
                    }
                } else {
                    self.ngroups += 1;
                    let idx = self.ngroups;
                    let n = self.alt()?;
                    self.expect_close()?;
                    Node::Group(Box::new(n), Some(idx))
                }
            }
            '\\' => {
                let e = self.peek().ok_or("bad escape (end of pattern)")?;
                self.p += 1;
                match e {
                    'A' => Node::StrStart,
                    'Z' | 'z' => Node::StrEnd,
                    'b' => Node::WordB(true),
                    'B' => Node::WordB(false),
                    'd' | 'D' | 'w' | 'W' | 's' | 'S' => Node::Class(vec![self.escape_class(e)?], false),
                    '1'..='9' => {
                        let mut n = e.to_digit(10).unwrap() as usize;
                        if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                            n = n * 10 + self.peek().unwrap().to_digit(10).unwrap() as usize;
                            self.p += 1;
                        }
                        Node::Backref(n)
                    }
                    _ => Node::Char(self.escape_char(e)?),
                }
            }
            ')' => return Err("unbalanced parenthesis".into()),
            '*' | '+' | '?' => return Err("nothing to repeat".into()),
            c => Node::Char(c),
        })
    }
    fn expect_close(&mut self) -> Result<(), PErr> {
        if self.eat(')') {
            Ok(())
        } else {
            Err("missing ), unterminated subpattern".into())
        }
    }
}

impl Node {
    fn global_marker(self) -> Node {
        Node::Empty
    }
}

// ---- program ----

#[derive(Clone, Debug)]
enum Inst {
    Char(char, bool),
    Any(bool),
    Class(usize, bool, bool),
    Bol(bool),
    Eol(bool),
    StrStart,
    StrEnd,
    WordB(bool, bool),
    Split(usize, usize),
    Jmp(usize),
    Save(usize),
    Backref(usize, bool),
    /// Sub-program [start, end) evaluated as an assertion.
    Look { start: usize, end: usize, ahead: bool, neg: bool, width: Option<usize> },
    Atomic { start: usize, end: usize },
    /// Repeat counter registers: set to 0, increment, test.
    RegSet(usize, i64),
    RegInc(usize),
    /// Jump to `.1` if reg < n.
    RegLt(usize, i64, usize),
    /// Progress check: fail if pos == reg (empty iteration).
    MarkPos(usize),
    CheckPos(usize),
    CondGroup(usize, usize),
    Match,
}

pub struct Program {
    insts: Vec<Inst>,
    classes: Vec<Vec<ClassItem>>,
    nregs: usize,
    pub ngroups: usize,
    pub names: Vec<(String, usize)>,
    pub flags: u32,
}

struct Compiler {
    insts: Vec<Inst>,
    classes: Vec<Vec<ClassItem>>,
    nregs: usize,
}

fn width(n: &Node) -> Option<usize> {
    Some(match n {
        Node::Empty | Node::Bol | Node::Eol | Node::StrStart | Node::StrEnd | Node::WordB(_) | Node::Look { .. } => 0,
        Node::Char(_) | Node::Any | Node::Class(..) => 1,
        Node::Group(n, _) | Node::Atomic(n) | Node::Flags(n, _, _) => width(n)?,
        Node::Concat(v) => {
            let mut s = 0;
            for x in v {
                s += width(x)?;
            }
            s
        }
        Node::Alt(v) => {
            let w = width(&v[0])?;
            for x in &v[1..] {
                if width(x)? != w {
                    return None;
                }
            }
            w
        }
        Node::Repeat { node, min, max: Some(mx), .. } if min == mx => width(node)? * *min as usize,
        _ => return None,
    })
}

fn can_be_empty(n: &Node) -> bool {
    match n {
        Node::Char(_) | Node::Any | Node::Class(..) => false,
        Node::Group(n, _) | Node::Atomic(n) | Node::Flags(n, _, _) => can_be_empty(n),
        Node::Concat(v) => v.iter().all(can_be_empty),
        Node::Alt(v) => v.iter().any(can_be_empty),
        Node::Repeat { node, min, .. } => *min == 0 || can_be_empty(node),
        _ => true,
    }
}

impl Compiler {
    fn emit(&mut self, i: Inst) -> usize {
        self.insts.push(i);
        self.insts.len() - 1
    }
    fn reg(&mut self) -> usize {
        self.nregs += 1;
        self.nregs - 1
    }
    fn node(&mut self, n: &Node, flags: u32) {
        let icase = flags & I != 0;
        match n {
            Node::Empty => {}
            Node::Char(c) => {
                self.emit(Inst::Char(*c, icase));
            }
            Node::Any => {
                self.emit(Inst::Any(flags & S != 0));
            }
            Node::Class(items, neg) => {
                self.classes.push(items.clone());
                let idx = self.classes.len() - 1;
                self.emit(Inst::Class(idx, *neg, icase));
            }
            Node::Bol => {
                self.emit(Inst::Bol(flags & M != 0));
            }
            Node::Eol => {
                self.emit(Inst::Eol(flags & M != 0));
            }
            Node::StrStart => {
                self.emit(Inst::StrStart);
            }
            Node::StrEnd => {
                self.emit(Inst::StrEnd);
            }
            Node::WordB(b) => {
                self.emit(Inst::WordB(*b, flags & A != 0));
            }
            Node::Group(inner, idx) => match idx {
                Some(i) => {
                    self.emit(Inst::Save(i * 2));
                    self.node(inner, flags);
                    self.emit(Inst::Save(i * 2 + 1));
                }
                None => self.node(inner, flags),
            },
            Node::Flags(inner, on, off) => self.node(inner, (flags | on) & !off),
            Node::Concat(v) => {
                for x in v {
                    self.node(x, flags);
                }
            }
            Node::Alt(v) => {
                let mut jumps = Vec::new();
                for (k, x) in v.iter().enumerate() {
                    if k + 1 < v.len() {
                        let split = self.emit(Inst::Split(0, 0));
                        self.node(x, flags);
                        jumps.push(self.emit(Inst::Jmp(0)));
                        let next = self.insts.len();
                        self.insts[split] = Inst::Split(split + 1, next);
                    } else {
                        self.node(x, flags);
                    }
                }
                let end = self.insts.len();
                for j in jumps {
                    self.insts[j] = Inst::Jmp(end);
                }
            }
            Node::Repeat { node, min, max, greedy, possessive } => {
                if *possessive {
                    let inner = Node::Repeat { node: node.clone(), min: *min, max: *max, greedy: true, possessive: false };
                    self.atomic(&inner, flags);
                    return;
                }
                self.repeat(node, *min, *max, *greedy, flags);
            }
            Node::Backref(g) => {
                self.emit(Inst::Backref(*g, icase));
            }
            Node::Look { node, ahead, neg } => {
                let w = if *ahead { None } else { width(node) };
                let at = self.emit(Inst::Look { start: 0, end: 0, ahead: *ahead, neg: *neg, width: w });
                let jmp = self.emit(Inst::Jmp(0));
                let start = self.insts.len();
                self.node(node, flags);
                self.emit(Inst::Match);
                let end = self.insts.len();
                self.insts[at] = Inst::Look { start, end, ahead: *ahead, neg: *neg, width: w };
                self.insts[jmp] = Inst::Jmp(end);
            }
            Node::Atomic(inner) => self.atomic(inner, flags),
            Node::Cond { group, yes, no } => {
                let c = self.emit(Inst::CondGroup(*group, 0));
                self.node(yes, flags);
                let j = self.emit(Inst::Jmp(0));
                let no_at = self.insts.len();
                self.node(no, flags);
                let end = self.insts.len();
                self.insts[c] = Inst::CondGroup(*group, no_at);
                self.insts[j] = Inst::Jmp(end);
            }
        }
    }
    fn atomic(&mut self, inner: &Node, flags: u32) {
        let at = self.emit(Inst::Atomic { start: 0, end: 0 });
        let jmp = self.emit(Inst::Jmp(0));
        let start = self.insts.len();
        self.node(inner, flags);
        self.emit(Inst::Match);
        let end = self.insts.len();
        self.insts[at] = Inst::Atomic { start, end };
        self.insts[jmp] = Inst::Jmp(end);
    }
    fn star(&mut self, node: &Node, greedy: bool, flags: u32) {
        // L: split(body, out); body; jmp L
        let empty = can_be_empty(node);
        let r = if empty { Some(self.reg()) } else { None };
        let l = self.emit(Inst::Split(0, 0));
        if let Some(r) = r {
            self.emit(Inst::MarkPos(r));
        }
        self.node(node, flags);
        if let Some(r) = r {
            self.emit(Inst::CheckPos(r));
        }
        self.emit(Inst::Jmp(l));
        let out = self.insts.len();
        self.insts[l] = if greedy { Inst::Split(l + 1, out) } else { Inst::Split(out, l + 1) };
    }
    fn optional(&mut self, node: &Node, greedy: bool, flags: u32) {
        let s = self.emit(Inst::Split(0, 0));
        self.node(node, flags);
        let out = self.insts.len();
        self.insts[s] = if greedy { Inst::Split(s + 1, out) } else { Inst::Split(out, s + 1) };
    }
    fn repeat(&mut self, node: &Node, min: u32, max: Option<u32>, greedy: bool, flags: u32) {
        let small = min <= 16 && max.map(|m| m <= 16).unwrap_or(true);
        if small {
            for _ in 0..min {
                self.node(node, flags);
            }
            match max {
                None => self.star(node, greedy, flags),
                Some(mx) => {
                    for _ in min..mx {
                        self.optional(node, greedy, flags);
                    }
                }
            }
            return;
        }
        // Counted loop with a register.
        let r = self.reg();
        self.emit(Inst::RegSet(r, 0));
        // Mandatory part.
        if min > 0 {
            let top = self.insts.len();
            self.node(node, flags);
            self.emit(Inst::RegInc(r));
            self.emit(Inst::RegLt(r, min as i64, top));
        }
        match max {
            None => self.star(node, greedy, flags),
            Some(mx) => {
                let r2 = self.reg();
                self.emit(Inst::RegSet(r2, min as i64));
                let top = self.insts.len();
                let test = self.emit(Inst::RegLt(r2, mx as i64, 0));
                let jout = self.emit(Inst::Jmp(0));
                let body = self.insts.len();
                let split = self.emit(Inst::Split(0, 0));
                self.node(node, flags);
                self.emit(Inst::RegInc(r2));
                self.emit(Inst::Jmp(top));
                let out = self.insts.len();
                self.insts[test] = Inst::RegLt(r2, mx as i64, body);
                self.insts[jout] = Inst::Jmp(out);
                self.insts[split] = if greedy { Inst::Split(split + 1, out) } else { Inst::Split(out, split + 1) };
            }
        }
    }
}

pub fn compile(pattern: &str, flags: u32) -> Result<Program, String> {
    let chars: Vec<char> = pattern.chars().collect();
    // Global inline flags anywhere at the top level apply to the whole pattern.
    let mut flags = flags;
    let mut i = 0;
    while i + 2 < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == '(' && chars[i + 1] == '?' {
            let mut j = i + 2;
            let mut f = 0;
            while j < chars.len() && "aiLmsux".contains(chars[j]) {
                f |= match chars[j] {
                    'i' => I,
                    'm' => M,
                    's' => S,
                    'x' => X,
                    'a' => A,
                    'u' => U,
                    _ => L,
                };
                j += 1;
            }
            if j > i + 2 && chars.get(j) == Some(&')') {
                flags |= f;
            }
        }
        i += 1;
    }
    let mut p = ReParser { c: &chars, p: 0, flags, ngroups: 0, names: Vec::new() };
    let node = p.alt()?;
    if p.p < chars.len() {
        return Err("unbalanced parenthesis".into());
    }
    let flags = p.flags;
    let mut c = Compiler { insts: Vec::new(), classes: Vec::new(), nregs: 0 };
    c.emit(Inst::Save(0));
    c.node(&node, flags);
    c.emit(Inst::Save(1));
    c.emit(Inst::Match);
    Ok(Program { insts: c.insts, classes: c.classes, nregs: c.nregs, ngroups: p.ngroups, names: p.names, flags })
}

fn is_word(c: char, ascii: bool) -> bool {
    if ascii { c.is_ascii_alphanumeric() || c == '_' } else { c.is_alphanumeric() || c == '_' }
}

fn fold(c: char) -> char {
    let mut l = c.to_lowercase();
    let f = l.next().unwrap_or(c);
    if l.next().is_some() { c } else { f }
}

fn class_match(items: &[ClassItem], c: char, icase: bool, ascii: bool) -> bool {
    let test = |c: char| {
        items.iter().any(|it| match it {
            ClassItem::Ch(x) => *x == c,
            ClassItem::Range(a, b) => *a <= c && c <= *b,
            ClassItem::Digit(neg) => (if ascii { c.is_ascii_digit() } else { c.is_ascii_digit() || (c.is_numeric() && c.to_digit(10).is_some()) || ('０'..='９').contains(&c) }) != *neg,
            ClassItem::Word(neg) => is_word(c, ascii) != *neg,
            ClassItem::Space(neg) => (if ascii { matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c') } else { c.is_whitespace() }) != *neg,
        })
    };
    if test(c) {
        return true;
    }
    if icase {
        let l = fold(c);
        let u = c.to_uppercase().next().unwrap_or(c);
        return test(l) || test(u);
    }
    false
}

enum Frame {
    Thread(usize, usize),
    Undo(usize, Option<usize>),
    UndoReg(usize, i64),
}

pub struct Matcher<'a> {
    prog: &'a Program,
    s: &'a [char],
    slots: Vec<Option<usize>>,
    regs: Vec<i64>,
    pub steps: u64,
}

impl<'a> Matcher<'a> {
    pub fn new(prog: &'a Program, s: &'a [char]) -> Self {
        Matcher { prog, s, slots: vec![None; (prog.ngroups + 1) * 2], regs: vec![0; prog.nregs], steps: 0 }
    }

    /// Runs instructions [pc..) at `pos`; returns the end position on Match.
    fn run(&mut self, pc0: usize, pos0: usize, end_limit: usize) -> Option<usize> {
        let mut stack: Vec<Frame> = vec![Frame::Thread(pc0, pos0)];
        let s = self.s;
        let ascii = self.prog.flags & A != 0;
        'threads: while let Some(f) = stack.pop() {
            let (mut pc, mut pos) = match f {
                Frame::Thread(pc, pos) => (pc, pos),
                Frame::Undo(slot, old) => {
                    self.slots[slot] = old;
                    continue;
                }
                Frame::UndoReg(r, old) => {
                    self.regs[r] = old;
                    continue;
                }
            };
            loop {
                self.steps += 1;
                if self.steps > 50_000_000 {
                    return None;
                }
                match &self.prog.insts[pc] {
                    Inst::Match => {
                        // Discard pending alternatives but keep slot values.
                        return Some(pos);
                    }
                    Inst::Char(c, icase) => {
                        if pos < end_limit && (s[pos] == *c || (*icase && fold(s[pos]) == fold(*c))) {
                            pos += 1;
                            pc += 1;
                        } else {
                            continue 'threads;
                        }
                    }
                    Inst::Any(dotall) => {
                        if pos < end_limit && (*dotall || s[pos] != '\n') {
                            pos += 1;
                            pc += 1;
                        } else {
                            continue 'threads;
                        }
                    }
                    Inst::Class(idx, neg, icase) => {
                        if pos < end_limit && class_match(&self.prog.classes[*idx], s[pos], *icase, ascii) != *neg {
                            pos += 1;
                            pc += 1;
                        } else {
                            continue 'threads;
                        }
                    }
                    Inst::Bol(multi) => {
                        if pos == 0 || (*multi && s[pos - 1] == '\n') {
                            pc += 1;
                        } else {
                            continue 'threads;
                        }
                    }
                    Inst::Eol(multi) => {
                        let n = s.len();
                        if pos == n || (pos + 1 == n && s[pos] == '\n') || (*multi && s[pos] == '\n') {
                            pc += 1;
                        } else {
                            continue 'threads;
                        }
                    }
                    Inst::StrStart => {
                        if pos == 0 {
                            pc += 1;
                        } else {
                            continue 'threads;
                        }
                    }
                    Inst::StrEnd => {
                        if pos == s.len() {
                            pc += 1;
                        } else {
                            continue 'threads;
                        }
                    }
                    Inst::WordB(want, asc) => {
                        let a = pos > 0 && is_word(s[pos - 1], *asc || ascii);
                        let b = pos < s.len() && is_word(s[pos], *asc || ascii);
                        if (a != b) == *want {
                            pc += 1;
                        } else {
                            continue 'threads;
                        }
                    }
                    Inst::Split(a, b) => {
                        stack.push(Frame::Thread(*b, pos));
                        pc = *a;
                    }
                    Inst::Jmp(a) => pc = *a,
                    Inst::Save(slot) => {
                        stack.push(Frame::Undo(*slot, self.slots[*slot]));
                        self.slots[*slot] = Some(pos);
                        pc += 1;
                    }
                    Inst::Backref(g, icase) => {
                        let (Some(a), Some(b)) = (self.slots.get(g * 2).copied().flatten(), self.slots.get(g * 2 + 1).copied().flatten()) else {
                            continue 'threads;
                        };
                        let n = b - a;
                        if pos + n > end_limit {
                            continue 'threads;
                        }
                        let ok = (0..n).all(|k| s[a + k] == s[pos + k] || (*icase && fold(s[a + k]) == fold(s[pos + k])));
                        if !ok {
                            continue 'threads;
                        }
                        pos += n;
                        pc += 1;
                    }
                    Inst::Look { start, end, ahead, neg, width } => {
                        let saved = self.slots.clone();
                        let ok = if *ahead {
                            self.run(*start, pos, s.len()).is_some()
                        } else {
                            match width {
                                Some(w) => pos >= *w && self.run(*start, pos - w, pos).is_some_and(|e| e == pos),
                                None => (0..=pos).rev().any(|st| self.run(*start, st, pos).is_some_and(|e| e == pos)),
                            }
                        };
                        if *neg || !ok {
                            self.slots = saved;
                        } else {
                            // Keep captures from a positive lookaround; undo on backtrack.
                            for (i, old) in saved.into_iter().enumerate() {
                                if self.slots[i] != old {
                                    stack.push(Frame::Undo(i, old));
                                }
                            }
                        }
                        if ok != *neg {
                            pc = *end;
                        } else {
                            continue 'threads;
                        }
                    }
                    Inst::Atomic { start, end } => {
                        let saved = self.slots.clone();
                        match self.run(*start, pos, end_limit) {
                            Some(e) => {
                                for (i, old) in saved.into_iter().enumerate() {
                                    if self.slots[i] != old {
                                        stack.push(Frame::Undo(i, old));
                                    }
                                }
                                pos = e;
                                pc = *end;
                            }
                            None => {
                                self.slots = saved;
                                continue 'threads;
                            }
                        }
                    }
                    Inst::RegSet(r, v) => {
                        stack.push(Frame::UndoReg(*r, self.regs[*r]));
                        self.regs[*r] = *v;
                        pc += 1;
                    }
                    Inst::RegInc(r) => {
                        stack.push(Frame::UndoReg(*r, self.regs[*r]));
                        self.regs[*r] += 1;
                        pc += 1;
                    }
                    Inst::RegLt(r, n, t) => {
                        pc = if self.regs[*r] < *n { *t } else { pc + 1 };
                    }
                    Inst::MarkPos(r) => {
                        stack.push(Frame::UndoReg(*r, self.regs[*r]));
                        self.regs[*r] = pos as i64;
                        pc += 1;
                    }
                    Inst::CheckPos(r) => {
                        if self.regs[*r] == pos as i64 {
                            continue 'threads;
                        }
                        pc += 1;
                    }
                    Inst::CondGroup(g, no) => {
                        let set = self.slots.get(g * 2 + 1).copied().flatten().is_some();
                        pc = if set { pc + 1 } else { *no };
                    }
                }
            }
        }
        None
    }

    /// Anchored match at `start` (end bounded by `endpos`).
    pub fn match_at(&mut self, start: usize, endpos: usize, full: bool) -> Option<Vec<Option<usize>>> {
        for v in self.slots.iter_mut() {
            *v = None;
        }
        if full {
            // Fullmatch: require the overall end to equal endpos by retrying
            // with an end assertion appended through the bound.
            let s_end = endpos;
            let mut found = None;
            let mut try_pos = Some(());
            while try_pos.take().is_some() {
                if let Some(e) = self.run(0, start, s_end) {
                    if e == s_end {
                        found = Some(self.slots.clone());
                    }
                }
            }
            if found.is_none() {
                // Retry with an explicit end anchor in a modified program.
                return None;
            }
            return found;
        }
        self.run(0, start, endpos).map(|_| self.slots.clone())
    }
}

// ---- Python objects ----

pub struct PatternData {
    pub pattern: Value,
    pub prog: Rc<Program>,
    pub full_prog: Rc<Program>,
    pub flags: u32,
    pub bytes: bool,
}

pub struct MatchData {
    pub string: Value,
    pub chars: Rc<Vec<char>>,
    pub slots: Vec<Option<usize>>,
    pub pattern: Value,
    pub pos: usize,
    pub endpos: usize,
    pub names: Vec<(String, usize)>,
    pub bytes: bool,
}

fn re_error(vm: &Vm, msg: String) -> crate::vm::PyErr {
    let cls = crate::import::get_module(vm, "_sre").and_then(|m| m.obj().and_then(|o| o.dict_get_str_raw("error")));
    let cls = match cls {
        Some(Value::Obj(c)) => c,
        _ => vm.types.exc("Exception"),
    };
    let m = vm.types.new_string(msg);
    crate::vm::PyErr::Exc(vm.new_exc(&cls, vec![m]))
}

fn text_of(vm: &Vm, v: &Value) -> PyResult<(Vec<char>, bool)> {
    if let Some(s) = v.as_str() {
        return Ok((s.chars().collect(), false));
    }
    if let Some(b) = ops::bytes_of(v) {
        return Ok((b.iter().map(|c| *c as char).collect(), true));
    }
    Err(vm.type_error(format!("expected string or bytes-like object, got '{}'", ops::type_name(vm, v))))
}

fn mk_str(vm: &Vm, chars: &[char], bytes: bool) -> Value {
    if bytes {
        vm.types.new_bytes(chars.iter().map(|c| *c as u8).collect())
    } else {
        vm.types.new_string(chars.iter().collect())
    }
}

fn pattern_class(vm: &Vm) -> ObjRef {
    vm.classes.pattern.clone().unwrap()
}

pub fn compile_pattern(vm: &mut Vm, pat: &Value, flags: u32) -> PyResult<Value> {
    if let Some(Payload::Native(n)) = pat.payload() {
        if n.borrow().is::<PatternData>() {
            return Ok(pat.clone());
        }
    }
    let (chars, bytes) = text_of(vm, pat)?;
    let src: String = chars.iter().collect();
    let key = (src.clone(), flags, bytes);
    if let Some(cache) = vm.ext_get::<ReCache>() {
        if let Some(v) = cache.0.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone()) {
            return Ok(v);
        }
    }
    let flags = if bytes { flags } else { flags | if flags & A == 0 { U } else { 0 } };
    let prog = compile(&src, flags).map_err(|e| re_error(vm, format!("{e} at position 0")))?;
    let full = compile(&format!("(?:{src})\\Z"), flags).map_err(|e| re_error(vm, e))?;
    let fl = prog.flags;
    let pd = PatternData { pattern: pat.clone(), prog: Rc::new(prog), full_prog: Rc::new(full), flags: fl, bytes };
    let v = Value::Obj(new_obj(pattern_class(vm), Some(Dict::new()), Payload::Native(std::cell::RefCell::new(Box::new(pd)))));
    if vm.ext_get::<ReCache>().is_none() {
        vm.ext_set(ReCache(Vec::new()));
    }
    if let Some(mut c) = vm.ext_get::<ReCache>() {
        if c.0.len() > 512 {
            c.0.clear();
        }
        c.0.push((key, v.clone()));
    }
    Ok(v)
}

struct ReCache(Vec<((String, u32, bool), Value)>);

fn pat_data(vm: &Vm, v: &Value) -> PyResult<(Rc<Program>, Rc<Program>, bool, u32, Value)> {
    match v.payload() {
        Some(Payload::Native(n)) => match n.borrow().downcast_ref::<PatternData>() {
            Some(p) => Ok((p.prog.clone(), p.full_prog.clone(), p.bytes, p.flags, p.pattern.clone())),
            None => Err(vm.type_error("Pattern expected")),
        },
        _ => Err(vm.type_error("Pattern expected")),
    }
}

fn make_match(vm: &Vm, pat: &Value, string: &Value, chars: Rc<Vec<char>>, slots: Vec<Option<usize>>, pos: usize, endpos: usize, prog: &Program, bytes: bool) -> Value {
    let md = MatchData { string: string.clone(), chars, slots, pattern: pat.clone(), pos, endpos, names: prog.names.clone(), bytes };
    Value::Obj(new_obj(vm.classes.match_.clone().unwrap(), None, Payload::Native(std::cell::RefCell::new(Box::new(md)))))
}

fn bounds(vm: &mut Vm, a: &[Value], k: Kw, n: usize) -> PyResult<(usize, usize)> {
    let [_, pos, endpos] = parse(vm, "match", a, k, ["string", "pos", "endpos"], 1)?;
    let p = crate::builtins::opt_i64(vm, &pos, 0)?.clamp(0, n as i64) as usize;
    let e = crate::builtins::opt_i64(vm, &endpos, n as i64)?.clamp(0, n as i64) as usize;
    Ok((p, e.max(p)))
}

fn search_impl(vm: &mut Vm, pat: &Value, string: &Value, pos: usize, endpos: usize, mode: u8) -> PyResult<Value> {
    let (prog, full, bytes, _, _) = pat_data(vm, pat)?;
    let (chars, _) = text_of(vm, string)?;
    let chars = Rc::new(chars);
    let p = if mode == 2 { full.clone() } else { prog.clone() };
    let mut m = Matcher::new(&p, &chars);
    let starts: Box<dyn Iterator<Item = usize>> = if mode == 0 { Box::new(pos..=endpos) } else { Box::new(std::iter::once(pos)) };
    for st in starts {
        let r = if mode == 2 { m.match_at(st, endpos, false).filter(|sl| sl[1] == Some(endpos)) } else { m.match_at(st, endpos, false) };
        if let Some(mut slots) = r {
            if mode == 2 {
                slots[1] = Some(endpos);
            }
            return Ok(make_match(vm, pat, string, chars.clone(), slots, pos, endpos, &prog, bytes));
        }
    }
    Ok(Value::None)
}

fn p_match(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let (chars, _) = text_of(vm, &a[1])?;
    let (p, e) = bounds(vm, &a[1..], k, chars.len())?;
    search_impl(vm, &a[0], &a[1], p, e, 1)
}
fn p_search(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let (chars, _) = text_of(vm, &a[1])?;
    let (p, e) = bounds(vm, &a[1..], k, chars.len())?;
    search_impl(vm, &a[0], &a[1], p, e, 0)
}
fn p_fullmatch(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let (chars, _) = text_of(vm, &a[1])?;
    let (p, e) = bounds(vm, &a[1..], k, chars.len())?;
    search_impl(vm, &a[0], &a[1], p, e, 2)
}

/// All non-overlapping matches as slot vectors.
fn all_matches(vm: &mut Vm, pat: &Value, chars: &[char], pos: usize, endpos: usize) -> PyResult<Vec<Vec<Option<usize>>>> {
    let (prog, _, _, _, _) = pat_data(vm, pat)?;
    let mut m = Matcher::new(&prog, chars);
    let mut out = Vec::new();
    let mut at = pos;
    let mut last_empty_end: Option<usize> = None;
    while at <= endpos {
        let mut found = None;
        for st in at..=endpos {
            if let Some(sl) = m.match_at(st, endpos, false) {
                if sl[0] == sl[1] && last_empty_end == sl[0] && out.last().map(|l: &Vec<Option<usize>>| l[1]) == Some(sl[0]) {
                    continue;
                }
                found = Some(sl);
                break;
            }
        }
        let Some(sl) = found else { break };
        let (s0, e0) = (sl[0].unwrap(), sl[1].unwrap());
        out.push(sl);
        if e0 == s0 {
            last_empty_end = Some(e0);
            at = e0 + 1;
            // An empty match right after a non-empty one is allowed at e0.
        } else {
            last_empty_end = None;
            at = e0;
        }
    }
    Ok(out)
}

fn group_value(vm: &Vm, chars: &[char], slots: &[Option<usize>], g: usize, bytes: bool, default: &Value) -> Value {
    match (slots.get(g * 2).copied().flatten(), slots.get(g * 2 + 1).copied().flatten()) {
        (Some(a), Some(b)) => mk_str(vm, &chars[a..b], bytes),
        _ => default.clone(),
    }
}

fn p_findall(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let (chars, bytes) = text_of(vm, &a[1])?;
    let (p, e) = bounds(vm, &a[1..], k, chars.len())?;
    let (prog, _, _, _, _) = pat_data(vm, &a[0])?;
    let ms = all_matches(vm, &a[0], &chars, p, e)?;
    let empty = mk_str(vm, &[], bytes);
    let items = ms
        .iter()
        .map(|sl| match prog.ngroups {
            0 => group_value(vm, &chars, sl, 0, bytes, &empty),
            1 => group_value(vm, &chars, sl, 1, bytes, &empty),
            n => {
                let g = (1..=n).map(|i| group_value(vm, &chars, sl, i, bytes, &empty)).collect();
                vm.types.new_tuple(g)
            }
        })
        .collect();
    Ok(vm.types.new_list(items))
}

fn p_finditer(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let (chars, bytes) = text_of(vm, &a[1])?;
    let (p, e) = bounds(vm, &a[1..], k, chars.len())?;
    let (prog, _, _, _, _) = pat_data(vm, &a[0])?;
    let ms = all_matches(vm, &a[0], &chars, p, e)?;
    let rc = Rc::new(chars);
    let items = ms.into_iter().map(|sl| make_match(vm, &a[0], &a[1], rc.clone(), sl, p, e, &prog, bytes)).collect();
    Ok(vm.types.new_iter(IterState::Values { items, idx: 0 }))
}

fn p_split(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [string, maxsplit] = parse(vm, "split", &a[1..], k, ["string", "maxsplit"], 1)?;
    let string = string.unwrap();
    let (chars, bytes) = text_of(vm, &string)?;
    let max = crate::builtins::opt_i64(vm, &maxsplit, 0)?;
    let (prog, _, _, _, _) = pat_data(vm, &a[0])?;
    let ms = all_matches(vm, &a[0], &chars, 0, chars.len())?;
    let mut out = Vec::new();
    let mut last = 0;
    for (n, sl) in ms.iter().enumerate() {
        if max > 0 && n as i64 >= max {
            break;
        }
        let (s0, e0) = (sl[0].unwrap(), sl[1].unwrap());
        out.push(mk_str(vm, &chars[last..s0], bytes));
        for g in 1..=prog.ngroups {
            out.push(group_value(vm, &chars, sl, g, bytes, &Value::None));
        }
        last = e0;
    }
    out.push(mk_str(vm, &chars[last..], bytes));
    Ok(vm.types.new_list(out))
}

/// Expands a replacement template for one match.
fn expand_template(vm: &mut Vm, tmpl: &[char], chars: &[char], slots: &[Option<usize>], names: &[(String, usize)], ngroups: usize, bytes: bool) -> PyResult<String> {
    let mut out = String::new();
    let mut i = 0;
    let group = |vm: &Vm, g: usize| -> PyResult<String> {
        if g > ngroups {
            return Err(re_error(vm, format!("invalid group reference {g}")));
        }
        Ok(match (slots.get(g * 2).copied().flatten(), slots.get(g * 2 + 1).copied().flatten()) {
            (Some(a), Some(b)) => chars[a..b].iter().collect(),
            _ => String::new(),
        })
    };
    let _ = bytes;
    while i < tmpl.len() {
        let c = tmpl[i];
        if c != '\\' || i + 1 >= tmpl.len() {
            out.push(c);
            i += 1;
            continue;
        }
        let n = tmpl[i + 1];
        i += 2;
        match n {
            'g' => {
                if tmpl.get(i) != Some(&'<') {
                    return Err(re_error(vm, "missing <".into()));
                }
                let st = i + 1;
                let end = tmpl[st..].iter().position(|c| *c == '>').map(|p| p + st).ok_or_else(|| re_error(vm, "missing >".into()))?;
                let name: String = tmpl[st..end].iter().collect();
                i = end + 1;
                let g = match name.parse::<usize>() {
                    Ok(g) => g,
                    Err(_) => names.iter().find(|(n, _)| *n == name).map(|(_, g)| *g).ok_or_else(|| vm.index_error(format!("unknown group name '{name}'")))?,
                };
                out.push_str(&group(vm, g)?);
            }
            '0'..='9' => {
                let mut g = n.to_digit(10).unwrap() as usize;
                if tmpl.get(i).is_some_and(|c| c.is_ascii_digit()) {
                    g = g * 10 + tmpl[i].to_digit(10).unwrap() as usize;
                    i += 1;
                }
                if n == '0' {
                    out.push(char::from_u32(g as u32).unwrap_or('\0'));
                } else {
                    out.push_str(&group(vm, g)?);
                }
            }
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'f' => out.push('\x0c'),
            'v' => out.push('\x0b'),
            'a' => out.push('\x07'),
            'b' => out.push('\x08'),
            '\\' => out.push('\\'),
            c if c.is_ascii_alphabetic() => return Err(re_error(vm, format!("bad escape \\{c}"))),
            c => {
                out.push('\\');
                out.push(c);
            }
        }
    }
    Ok(out)
}

fn sub_impl(vm: &mut Vm, a: &[Value], k: Kw, want_count: bool) -> PyResult<Value> {
    let [repl, string, count] = parse(vm, "sub", &a[1..], k, ["repl", "string", "count"], 2)?;
    let (repl, string) = (repl.unwrap(), string.unwrap());
    let (chars, bytes) = text_of(vm, &string)?;
    let count = crate::builtins::opt_i64(vm, &count, 0)?;
    let (prog, _, _, _, _) = pat_data(vm, &a[0])?;
    let ms = all_matches(vm, &a[0], &chars, 0, chars.len())?;
    let callable = repl.as_str().is_none() && ops::bytes_of(&repl).is_none();
    let tmpl: Vec<char> = if callable { Vec::new() } else { text_of(vm, &repl)?.0 };
    let literal = !callable && !tmpl.contains(&'\\');
    let rc = Rc::new(chars.clone());
    let mut out = String::new();
    let mut last = 0;
    let mut n = 0i64;
    for sl in ms {
        if count > 0 && n >= count {
            break;
        }
        let (s0, e0) = (sl[0].unwrap(), sl[1].unwrap());
        out.extend(&chars[last..s0]);
        if callable {
            let m = make_match(vm, &a[0], &string, rc.clone(), sl.clone(), 0, chars.len(), &prog, bytes);
            let r = vm.call(&repl, vec![m], Vec::new())?;
            let (rs, _) = text_of(vm, &r)?;
            out.extend(rs);
        } else if literal {
            out.extend(tmpl.iter());
        } else {
            let r = expand_template(vm, &tmpl, &chars, &sl, &prog.names, prog.ngroups, bytes)?;
            out.push_str(&r);
        }
        last = e0;
        n += 1;
    }
    out.extend(&chars[last..]);
    let s: Vec<char> = out.chars().collect();
    let r = mk_str(vm, &s, bytes);
    if want_count {
        return Ok(vm.types.new_tuple(vec![r, Value::Int(n)]));
    }
    Ok(r)
}

fn p_sub(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    sub_impl(vm, a, k, false)
}
fn p_subn(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    sub_impl(vm, a, k, true)
}

fn pattern_attr(vm: &mut Vm, o: &Value, key: &str) -> PyResult<Option<Value>> {
    let (prog, _, _, flags, pat) = pat_data(vm, o)?;
    Ok(Some(match key {
        "pattern" => pat,
        "flags" => Value::Int(flags as i64),
        "groups" => Value::Int(prog.ngroups as i64),
        "groupindex" => {
            let d = vm.types.new_dict_obj(Dict::new());
            for (n, i) in &prog.names {
                let k = vm.types.new_str(n);
                ops::dict_set(vm, &d, k, Value::Int(*i as i64))?;
            }
            Value::Obj(d)
        }
        _ => return Ok(None),
    }))
}

fn p_getattr(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let key = a[1].as_str().unwrap_or("").to_string();
    match pattern_attr(vm, &a[0], &key)? {
        Some(v) => Ok(v),
        None => Err(vm.attr_error(format!("'re.Pattern' object has no attribute '{key}'"))),
    }
}

fn p_repr(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let (_, _, _, _, pat) = pat_data(vm, &a[0])?;
    let r = ops::repr(vm, &pat)?;
    Ok(vm.types.new_string(format!("re.compile({r})")))
}

fn p_reduce(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let (_, _, _, flags, pat) = pat_data(vm, &a[0])?;
    let m = crate::import::import_module(vm, "_sre")?;
    let c = ops::getattr(vm, &m, "compile")?;
    let args = vm.types.new_tuple(vec![pat, Value::Int((flags & !U) as i64)]);
    Ok(vm.types.new_tuple(vec![c, args]))
}

// ---- match objects ----

fn md<R>(vm: &Vm, v: &Value, f: impl FnOnce(&MatchData) -> R) -> PyResult<R> {
    match v.payload() {
        Some(Payload::Native(n)) => n.borrow().downcast_ref::<MatchData>().map(f).ok_or_else(|| vm.type_error("Match expected")),
        _ => Err(vm.type_error("Match expected")),
    }
}

fn group_index(vm: &mut Vm, m: &Value, g: &Value) -> PyResult<usize> {
    let (names, ng) = md(vm, m, |d| (d.names.clone(), d.slots.len() / 2 - 1))?;
    let idx = if let Some(s) = g.as_str() {
        names.iter().find(|(n, _)| n == s).map(|(_, i)| *i).ok_or_else(|| vm.index_error("no such group"))?
    } else {
        let i = ops::index_of(vm, g)?;
        if i < 0 {
            return Err(vm.index_error("no such group"));
        }
        i as usize
    };
    if idx > ng {
        return Err(vm.index_error("no such group"));
    }
    Ok(idx)
}

fn m_group_one(vm: &mut Vm, m: &Value, g: &Value, default: &Value) -> PyResult<Value> {
    let idx = group_index(vm, m, g)?;
    let (chars, slots, bytes) = md(vm, m, |d| (d.chars.clone(), d.slots.clone(), d.bytes))?;
    Ok(group_value(vm, &chars, &slots, idx, bytes, default))
}

fn m_group(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    if a.len() <= 2 {
        let g = a.get(1).cloned().unwrap_or(Value::Int(0));
        return m_group_one(vm, &a[0], &g, &Value::None);
    }
    let mut out = Vec::new();
    for g in &a[1..] {
        out.push(m_group_one(vm, &a[0], g, &Value::None)?);
    }
    Ok(vm.types.new_tuple(out))
}

fn m_getitem(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    m_group_one(vm, &a[0], &a[1], &Value::None)
}

fn m_groups(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [default] = parse(vm, "groups", &a[1..], k, ["default"], 0)?;
    let default = default.unwrap_or(Value::None);
    let (chars, slots, bytes) = md(vm, &a[0], |d| (d.chars.clone(), d.slots.clone(), d.bytes))?;
    let n = slots.len() / 2;
    let items = (1..n).map(|g| group_value(vm, &chars, &slots, g, bytes, &default)).collect();
    Ok(vm.types.new_tuple(items))
}

fn m_groupdict(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [default] = parse(vm, "groupdict", &a[1..], k, ["default"], 0)?;
    let default = default.unwrap_or(Value::None);
    let (chars, slots, bytes, names) = md(vm, &a[0], |d| (d.chars.clone(), d.slots.clone(), d.bytes, d.names.clone()))?;
    let d = vm.types.new_dict_obj(Dict::new());
    for (n, g) in names {
        let k = vm.types.new_string(n);
        let v = group_value(vm, &chars, &slots, g, bytes, &default);
        ops::dict_set(vm, &d, k, v)?;
    }
    Ok(Value::Obj(d))
}

fn m_span_of(vm: &mut Vm, a: &[Value]) -> PyResult<(i64, i64)> {
    let g = a.get(1).cloned().unwrap_or(Value::Int(0));
    let idx = group_index(vm, &a[0], &g)?;
    md(vm, &a[0], |d| match (d.slots[idx * 2], d.slots[idx * 2 + 1]) {
        (Some(s), Some(e)) => (s as i64, e as i64),
        _ => (-1, -1),
    })
}

fn m_start(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Int(m_span_of(vm, a)?.0))
}
fn m_end(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Int(m_span_of(vm, a)?.1))
}
fn m_span(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let (s, e) = m_span_of(vm, a)?;
    Ok(vm.types.new_tuple(vec![Value::Int(s), Value::Int(e)]))
}

fn m_expand(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let (tmpl, _) = text_of(vm, &a[1])?;
    let (chars, slots, bytes, names) = md(vm, &a[0], |d| (d.chars.clone(), d.slots.clone(), d.bytes, d.names.clone()))?;
    let ng = slots.len() / 2 - 1;
    let r = expand_template(vm, &tmpl, &chars, &slots, &names, ng, bytes)?;
    let rc: Vec<char> = r.chars().collect();
    Ok(mk_str(vm, &rc, bytes))
}

fn m_getattr(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let key = a[1].as_str().unwrap_or("").to_string();
    let r = md(vm, &a[0], |d| {
        let last = (1..d.slots.len() / 2).filter(|g| d.slots[g * 2 + 1].is_some()).max_by_key(|g| d.slots[g * 2 + 1]);
        (d.string.clone(), d.pattern.clone(), d.pos, d.endpos, last, d.names.clone())
    })?;
    Ok(match key.as_str() {
        "string" => r.0,
        "re" => r.1,
        "pos" => Value::Int(r.2 as i64),
        "endpos" => Value::Int(r.3 as i64),
        "lastindex" => r.4.map(|g| Value::Int(g as i64)).unwrap_or(Value::None),
        "lastgroup" => match r.4.and_then(|g| r.5.iter().find(|(_, i)| *i == g).map(|(n, _)| n.clone())) {
            Some(n) => vm.types.new_string(n),
            None => Value::None,
        },
        "regs" => {
            let spans = md(vm, &a[0], |d| (0..d.slots.len() / 2).map(|g| (d.slots[g * 2].map(|x| x as i64).unwrap_or(-1), d.slots[g * 2 + 1].map(|x| x as i64).unwrap_or(-1))).collect::<Vec<_>>())?;
            let items = spans.into_iter().map(|(s, e)| vm.types.new_tuple(vec![Value::Int(s), Value::Int(e)])).collect();
            vm.types.new_tuple(items)
        }
        _ => return Err(vm.attr_error(format!("'re.Match' object has no attribute '{key}'"))),
    })
}

fn m_bool(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Bool(true))
}

fn m_repr(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let (s, e) = m_span_of(vm, &a[..1])?;
    let g = m_group_one(vm, &a[0], &Value::Int(0), &Value::None)?;
    let gr = ops::repr(vm, &g)?;
    Ok(vm.types.new_string(format!("<re.Match object; span=({s}, {e}), match={gr}>")))
}

// ---- module functions ----

fn flags_arg(vm: &mut Vm, v: Option<&Value>) -> PyResult<u32> {
    match v {
        Some(f) if !f.is_none() => Ok(ops::index_of(vm, f)? as u32),
        _ => Ok(0),
    }
}

fn m_compile(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [pat, flags] = parse(vm, "compile", a, k, ["pattern", "flags"], 1)?;
    let f = flags_arg(vm, flags.as_ref())?;
    compile_pattern(vm, &pat.unwrap(), f)
}

macro_rules! modfn {
    ($($name:ident => $m:ident, $n:expr),* $(,)?) => {
        $(
            fn $name(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
                // (pattern, string/repl..., flags=)
                let mut args = a.to_vec();
                let mut flags = 0;
                let mut rest: Vec<(Rc<str>, Value)> = Vec::new();
                for (kk, v) in k {
                    if &**kk == "flags" {
                        flags = flags_arg(vm, Some(v))?;
                    } else {
                        rest.push((kk.clone(), v.clone()));
                    }
                }
                if args.len() > $n {
                    flags = flags_arg(vm, args.get($n))?;
                    args.truncate($n);
                }
                let p = compile_pattern(vm, &args[0], flags)?;
                args[0] = p;
                $m(vm, &args, &rest)
            }
        )*
    };
}

modfn! {
    f_match => p_match, 3, f_search => p_search, 3, f_fullmatch => p_fullmatch, 3, f_findall => p_findall, 3,
    f_finditer => p_finditer, 3, f_split => p_split, 3, f_sub => p_sub, 4, f_subn => p_subn, 4,
}

fn f_escape(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let (chars, bytes) = text_of(vm, &a[0])?;
    let mut out = Vec::new();
    for c in chars {
        if !(c.is_alphanumeric() || c == '_') && (c as u32) < 128 {
            out.push('\\');
        }
        out.push(c);
    }
    Ok(mk_str(vm, &out, bytes))
}

fn f_purge(vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    if let Some(mut c) = vm.ext_get::<ReCache>() {
        c.0.clear();
    }
    Ok(Value::None)
}

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "_sre", &[
        ("compile", m_compile), ("match", f_match), ("search", f_search), ("fullmatch", f_fullmatch), ("findall", f_findall),
        ("finditer", f_finditer), ("split", f_split), ("sub", f_sub), ("subn", f_subn), ("escape", f_escape), ("purge", f_purge),
    ]);
    let pc = vm.types.new_builtin_type("Pattern", &vm.types.object.clone(), BaseKind::Native, true);
    for (n, f) in [
        ("match", p_match as NativeFn), ("search", p_search), ("fullmatch", p_fullmatch), ("findall", p_findall), ("finditer", p_finditer),
        ("split", p_split), ("sub", p_sub), ("subn", p_subn), ("__getattr__", p_getattr), ("__repr__", p_repr), ("__reduce__", p_reduce),
    ] {
        vm.types.add_method(&pc, n, f);
    }
    let mn = vm.types.new_str("re");
    vm.types.set_attr(&pc, "__module__", mn.clone());
    vm.classes.pattern = Some(pc.clone());
    set(vm, &m, "Pattern", Value::Obj(pc));
    let mc = vm.types.new_builtin_type("Match", &vm.types.object.clone(), BaseKind::Native, true);
    for (n, f) in [
        ("group", m_group as NativeFn), ("groups", m_groups), ("groupdict", m_groupdict), ("start", m_start), ("end", m_end),
        ("span", m_span), ("expand", m_expand), ("__getitem__", m_getitem), ("__getattr__", m_getattr), ("__bool__", m_bool), ("__repr__", m_repr),
    ] {
        vm.types.add_method(&mc, n, f);
    }
    vm.types.set_attr(&mc, "__module__", mn);
    vm.classes.match_ = Some(mc.clone());
    set(vm, &m, "Match", Value::Obj(mc));
    let err = vm.types.new_builtin_type("error", &vm.types.exc("Exception"), BaseKind::Exception, false);
    set(vm, &m, "error", Value::Obj(err));
    for (n, v) in [("I", I), ("IGNORECASE", I), ("L", L), ("LOCALE", L), ("M", M), ("MULTILINE", M), ("S", S), ("DOTALL", S), ("U", U), ("UNICODE", U), ("X", X), ("VERBOSE", X), ("A", A), ("ASCII", A), ("DEBUG", 128), ("T", 1), ("TEMPLATE", 1)] {
        set(vm, &m, n, Value::Int(v as i64));
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn find(p: &str, s: &str) -> Option<(usize, usize)> {
        let prog = compile(p, U).unwrap();
        let chars: Vec<char> = s.chars().collect();
        let mut m = Matcher::new(&prog, &chars);
        for st in 0..=chars.len() {
            if let Some(sl) = m.match_at(st, chars.len(), false) {
                return Some((sl[0].unwrap(), sl[1].unwrap()));
            }
        }
        None
    }
    #[test]
    fn basics() {
        assert_eq!(find("a+b", "xaaab"), Some((1, 5)));
        assert_eq!(find("(?P<x>\\d+)-(?P=x)", "12-12"), Some((0, 5)));
        assert_eq!(find("a{2,3}?", "aaaa"), Some((0, 2)));
        assert_eq!(find("(?<=\\$)\\d+", "cost $42"), Some((6, 8)));
        assert_eq!(find("(?i)HELLO", "say hello"), Some((4, 9)));
        assert_eq!(find("(a*)*b", "aaab"), Some((0, 4)));
        assert_eq!(find("x{20}", &"x".repeat(25)), Some((0, 20)));
        assert_eq!(find("\\bfoo\\b", "a foo b"), Some((2, 5)));
        assert_eq!(find("[^\\]]+", "]]ab]"), Some((2, 4)));
    }
}
