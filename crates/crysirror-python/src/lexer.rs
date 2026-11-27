// SPDX-License-Identifier: MPL-2.0
//! Python tokenizer shared by the Python 2 and Python 3 dialects.
//!
//! Follows CPython's `tokenize` rules: logical lines, INDENT/DEDENT from a
//! stack of column widths (tabs advance to the next multiple of 8), implicit
//! joining inside brackets and backslash continuation. String literals keep
//! their prefix and raw body; escape decoding and f-string splitting happen in
//! the parser so that error locations can point at the original source.

use crate::Dialect;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Name(String),
    /// Integer literal text with underscores removed, plus its radix and a
    /// Python 2 `L` suffix flag.
    Int { digits: String, radix: u32, long: bool },
    Float(f64),
    Imag(f64),
    /// Prefix letters lowered (`r`, `u`, `b`, `f` combinations) and body
    /// between the quotes, unprocessed.
    Str { prefix: String, body: String, triple: bool },
    Op(&'static str),
    Newline,
    Indent,
    Dedent,
    EndMarker,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug)]
pub struct LexError {
    pub msg: String,
    pub line: u32,
    pub col: u32,
}

const OPS: &[&str] = &[
    // Longest first.
    "**=", "//=", ">>=", "<<=", "...", "!=", "%=", "&=", "*=", "**", "+=", "-=", "->", "//", "/=",
    ":=", "<<", "<=", "<>", "==", ">=", ">>", "@=", "^=", "|=", "!", "%", "&", "(", ")", "*", "+",
    ",", "-", ".", "/", ":", ";", "<", "=", ">", "@", "[", "]", "^", "{", "|", "}", "~", "`",
];

pub struct Lexer<'a> {
    src: &'a [char],
    pos: usize,
    line: u32,
    line_start: usize,
    dialect: Dialect,
    indents: Vec<usize>,
    depth: usize,
    out: Vec<Token>,
    at_line_start: bool,
}

pub fn tokenize(source: &str, dialect: Dialect) -> Result<Vec<Token>, LexError> {
    let chars: Vec<char> = source.chars().collect();
    let mut lx = Lexer {
        src: &chars,
        pos: 0,
        line: 1,
        line_start: 0,
        dialect,
        indents: vec![0],
        depth: 0,
        out: Vec::new(),
        at_line_start: true,
    };
    lx.run()?;
    Ok(lx.out)
}

fn is_id_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}
fn is_id_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric() || matches!(c, '\u{00B7}' | '\u{0387}' | '\u{1369}'..='\u{1371}' | '\u{19DA}')
        || (!c.is_ascii() && unicode_combining(c))
}
fn unicode_combining(c: char) -> bool {
    // Combining marks and connector punctuation commonly found in identifiers.
    matches!(c as u32, 0x0300..=0x036F | 0x0483..=0x0487 | 0x0591..=0x05BD | 0x0610..=0x061A
        | 0x064B..=0x065F | 0x0900..=0x0903 | 0x093A..=0x094F | 0x3099..=0x309A | 0xFE00..=0xFE0F
        | 0x203F..=0x2040 | 0xFF3F)
}

impl<'a> Lexer<'a> {
    fn err<T>(&self, msg: impl Into<String>) -> Result<T, LexError> {
        Err(LexError { msg: msg.into(), line: self.line, col: (self.pos - self.line_start) as u32 })
    }
    fn peek(&self, off: usize) -> Option<char> {
        self.src.get(self.pos + off).copied()
    }
    fn push(&mut self, tok: Tok, start: usize, line: u32, line_start: usize) {
        self.out.push(Token { tok, line, col: (start - line_start) as u32 });
    }
    fn newline(&mut self) {
        self.line += 1;
        self.line_start = self.pos;
    }

    fn run(&mut self) -> Result<(), LexError> {
        loop {
            if self.at_line_start && self.depth == 0 {
                if !self.indentation()? {
                    continue;
                }
            }
            let Some(c) = self.peek(0) else { break };
            let start = self.pos;
            let (line, ls) = (self.line, self.line_start);
            match c {
                ' ' | '\t' | '\x0c' => {
                    self.pos += 1;
                }
                '#' => {
                    while let Some(c) = self.peek(0) {
                        if c == '\n' || c == '\r' {
                            break;
                        }
                        self.pos += 1;
                    }
                }
                '\r' | '\n' => {
                    self.pos += 1;
                    if c == '\r' && self.peek(0) == Some('\n') {
                        self.pos += 1;
                    }
                    if self.depth == 0 {
                        self.push(Tok::Newline, start, line, ls);
                        self.at_line_start = true;
                    }
                    self.newline();
                }
                '\\' => {
                    self.pos += 1;
                    match self.peek(0) {
                        Some('\n') => {
                            self.pos += 1;
                            self.newline();
                        }
                        Some('\r') => {
                            self.pos += 1;
                            if self.peek(0) == Some('\n') {
                                self.pos += 1;
                            }
                            self.newline();
                        }
                        None => {}
                        _ => return self.err("unexpected character after line continuation character"),
                    }
                }
                c if c.is_ascii_digit() || (c == '.' && self.peek(1).is_some_and(|d| d.is_ascii_digit())) => {
                    let tok = self.number()?;
                    self.push(tok, start, line, ls);
                }
                c if is_id_start(c) => {
                    let mut end = self.pos;
                    while end < self.src.len() && is_id_continue(self.src[end]) {
                        end += 1;
                    }
                    let word: String = self.src[self.pos..end].iter().collect();
                    let next = self.src.get(end).copied();
                    if matches!(next, Some('\'') | Some('"')) && is_string_prefix(&word, self.dialect) {
                        self.pos = end;
                        let tok = self.string(word.to_ascii_lowercase())?;
                        self.push(tok, start, line, ls);
                    } else {
                        self.pos = end;
                        self.push(Tok::Name(nfkc_ident(word)), start, line, ls);
                    }
                }
                '\'' | '"' => {
                    let tok = self.string(String::new())?;
                    self.push(tok, start, line, ls);
                }
                _ => {
                    let mut matched = None;
                    for op in OPS {
                        let n = op.chars().count();
                        if self.pos + n <= self.src.len()
                            && self.src[self.pos..self.pos + n].iter().copied().eq(op.chars())
                        {
                            matched = Some(*op);
                            break;
                        }
                    }
                    let Some(op) = matched else {
                        return self.err(format!("invalid character '{c}' (U+{:04X})", c as u32));
                    };
                    if op == "<>" && self.dialect == Dialect::Py3 {
                        // Only `<` here; `<>` is the Python 2 inequality.
                        self.pos += 1;
                        self.push(Tok::Op("<"), start, line, ls);
                        continue;
                    }
                    self.pos += op.chars().count();
                    match op {
                        "(" | "[" | "{" => self.depth += 1,
                        ")" | "]" | "}" => self.depth = self.depth.saturating_sub(1),
                        _ => {}
                    }
                    self.push(Tok::Op(op), start, line, ls);
                }
            }
        }
        let (line, ls) = (self.line, self.line_start);
        let end = self.pos;
        if !matches!(self.out.last().map(|t| &t.tok), None | Some(Tok::Newline) | Some(Tok::Dedent)) {
            self.push(Tok::Newline, end, line, ls);
        }
        while self.indents.len() > 1 {
            self.indents.pop();
            self.push(Tok::Dedent, end, line, ls);
        }
        self.push(Tok::EndMarker, end, line, ls);
        Ok(())
    }

    /// Measures indentation at a physical line start. Returns false when the
    /// line was blank or a comment (consumed with its newline).
    fn indentation(&mut self) -> Result<bool, LexError> {
        let mut col = 0usize;
        let mut p = self.pos;
        while let Some(&c) = self.src.get(p) {
            match c {
                ' ' => col += 1,
                '\t' => col = (col / 8 + 1) * 8,
                '\x0c' => col = 0,
                _ => break,
            }
            p += 1;
        }
        match self.src.get(p) {
            None => {
                self.pos = p;
                self.at_line_start = false;
                return Ok(true);
            }
            Some('#') | Some('\n') | Some('\r') => {
                // Blank or comment-only line: no tokens.
                self.pos = p;
                while let Some(c) = self.peek(0) {
                    if c == '\n' || c == '\r' {
                        break;
                    }
                    self.pos += 1;
                }
                if let Some(c) = self.peek(0) {
                    self.pos += 1;
                    if c == '\r' && self.peek(0) == Some('\n') {
                        self.pos += 1;
                    }
                    self.newline();
                }
                return Ok(false);
            }
            Some('\\') if matches!(self.src.get(p + 1), Some('\n') | Some('\r')) => {
                // A continuation at the start of a line joins with the next one.
            }
            _ => {}
        }
        self.pos = p;
        self.at_line_start = false;
        let (line, ls) = (self.line, self.line_start);
        let cur = *self.indents.last().unwrap();
        if col > cur {
            self.indents.push(col);
            self.push(Tok::Indent, p, line, ls);
        } else {
            while col < *self.indents.last().unwrap() {
                self.indents.pop();
                self.push(Tok::Dedent, p, line, ls);
            }
            if col != *self.indents.last().unwrap() {
                return self.err("unindent does not match any outer indentation level");
            }
        }
        Ok(true)
    }

    fn number(&mut self) -> Result<Tok, LexError> {
        let s = self.src;
        let start = self.pos;
        let take = |p: &mut usize, f: &dyn Fn(char) -> bool| {
            while *p < s.len() && (f(s[*p]) || s[*p] == '_') {
                *p += 1;
            }
        };
        let mut p = self.pos;
        if s[p] == '0' && p + 1 < s.len() && matches!(s[p + 1], 'x' | 'X' | 'o' | 'O' | 'b' | 'B') {
            let radix = match s[p + 1] {
                'x' | 'X' => 16,
                'o' | 'O' => 8,
                _ => 2,
            };
            p += 2;
            let ds = p;
            take(&mut p, &|c: char| c.is_digit(radix));
            let digits: String = s[ds..p].iter().filter(|c| **c != '_').collect();
            let long = p < s.len() && matches!(s[p], 'l' | 'L') && self.dialect == Dialect::Py2;
            if long {
                p += 1;
            }
            self.pos = p;
            if digits.is_empty() {
                return self.err("invalid number literal");
            }
            return Ok(Tok::Int { digits, radix, long });
        }
        take(&mut p, &|c: char| c.is_ascii_digit());
        let mut is_float = false;
        if p < s.len() && s[p] == '.' {
            is_float = true;
            p += 1;
            take(&mut p, &|c: char| c.is_ascii_digit());
        }
        if p < s.len() && matches!(s[p], 'e' | 'E') {
            let mut q = p + 1;
            if q < s.len() && matches!(s[q], '+' | '-') {
                q += 1;
            }
            if q < s.len() && s[q].is_ascii_digit() {
                is_float = true;
                p = q;
                take(&mut p, &|c: char| c.is_ascii_digit());
            }
        }
        let text: String = s[start..p].iter().filter(|c| **c != '_').collect();
        if p < s.len() && matches!(s[p], 'j' | 'J') {
            self.pos = p + 1;
            return Ok(Tok::Imag(text.parse().unwrap_or(0.0)));
        }
        if is_float {
            self.pos = p;
            return text.parse().map(Tok::Float).or_else(|_| self.err("invalid float literal"));
        }
        let long = p < s.len() && matches!(s[p], 'l' | 'L') && self.dialect == Dialect::Py2;
        if long {
            p += 1;
        }
        self.pos = p;
        // Python 2 leading-zero octal.
        if text.len() > 1 && text.starts_with('0') && text.chars().any(|c| c != '0') {
            if self.dialect == Dialect::Py2 {
                return Ok(Tok::Int { digits: text[1..].to_string(), radix: 8, long });
            }
            return self.err("leading zeros in decimal integer literals are not permitted");
        }
        Ok(Tok::Int { digits: text, radix: 10, long })
    }

    fn string(&mut self, prefix: String) -> Result<Tok, LexError> {
        let q = self.src[self.pos];
        let triple = self.peek(1) == Some(q) && self.peek(2) == Some(q);
        self.pos += if triple { 3 } else { 1 };
        let raw = prefix.contains('r');
        let fstr = prefix.contains('f');
        let mut body = String::new();
        let mut brace = 0usize;
        loop {
            let Some(c) = self.peek(0) else {
                return self.err("EOF while scanning string literal");
            };
            if brace == 0 || !fstr {
                if c == q {
                    if !triple {
                        self.pos += 1;
                        break;
                    }
                    if self.peek(1) == Some(q) && self.peek(2) == Some(q) {
                        self.pos += 3;
                        break;
                    }
                }
            }
            if c == '\\' {
                body.push(c);
                self.pos += 1;
                if let Some(n) = self.peek(0) {
                    body.push(n);
                    self.pos += 1;
                    if n == '\n' {
                        self.newline();
                    } else if n == '\r' {
                        if self.peek(0) == Some('\n') {
                            body.push('\n');
                            self.pos += 1;
                        }
                        self.newline();
                    }
                }
                let _ = raw;
                continue;
            }
            if c == '\n' || c == '\r' {
                if !triple && brace == 0 {
                    return self.err("EOL while scanning string literal");
                }
                self.pos += 1;
                if c == '\r' && self.peek(0) == Some('\n') {
                    self.pos += 1;
                }
                body.push('\n');
                self.newline();
                continue;
            }
            if fstr {
                if c == '{' {
                    if brace == 0 && self.peek(1) == Some('{') {
                        body.push_str("{{");
                        self.pos += 2;
                        continue;
                    }
                    brace += 1;
                } else if c == '}' && brace > 0 {
                    brace -= 1;
                } else if brace > 0 && (c == '\'' || c == '"') {
                    // A nested string inside a replacement field (PEP 701 permits
                    // the same quote character).
                    let start = self.pos;
                    let inner = self.string(String::new())?;
                    let _ = inner;
                    body.extend(self.src[start..self.pos].iter());
                    continue;
                }
            }
            body.push(c);
            self.pos += 1;
        }
        Ok(Tok::Str { prefix, body, triple })
    }
}

fn is_string_prefix(w: &str, d: Dialect) -> bool {
    let w = w.to_ascii_lowercase();
    match w.as_str() {
        "r" | "u" | "b" | "br" | "rb" => true,
        "f" | "fr" | "rf" => d == Dialect::Py3,
        "ur" => d == Dialect::Py2,
        _ => false,
    }
}

/// Identifiers are NFKC-normalised by CPython. Only the common compatibility
/// forms that occur in scripts (full-width ASCII) are folded here.
fn nfkc_ident(w: String) -> String {
    if w.is_ascii() {
        return w;
    }
    w.chars()
        .map(|c| match c as u32 {
            0xFF01..=0xFF5E => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            _ => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indents_and_brackets() {
        let t = tokenize("if x:\n    y = (1,\n  2)\nz\n", Dialect::Py3).unwrap();
        let kinds: Vec<_> = t.iter().map(|t| t.tok.clone()).collect();
        assert!(kinds.contains(&Tok::Indent));
        assert!(kinds.contains(&Tok::Dedent));
        assert_eq!(kinds.iter().filter(|k| **k == Tok::Newline).count(), 3);
    }
    #[test]
    fn fstring_nested_quotes() {
        let t = tokenize("f\"{x[\"a\"]}\"", Dialect::Py3).unwrap();
        assert!(matches!(&t[0].tok, Tok::Str { prefix, body, .. } if prefix == "f" && body == "{x[\"a\"]}"));
    }
    #[test]
    fn py2_octal_long() {
        let t = tokenize("0777 10L", Dialect::Py2).unwrap();
        assert!(matches!(&t[0].tok, Tok::Int { radix: 8, .. }));
        assert!(matches!(&t[1].tok, Tok::Int { long: true, .. }));
    }
}
