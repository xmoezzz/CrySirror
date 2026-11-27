// SPDX-License-Identifier: MPL-2.0
//! Ren'Py script lexer (`renpy/lexer.py`): logical lines, indentation
//! blocks and the statement-level token API used by the built-in parser and
//! by creator-defined statement parse functions.
//!
//! Regular expressions passed to `match_re` use Python syntax and run on the
//! interpreter's `_sre` engine, so creator-defined statements that call
//! `l.match(...)` behave as in Ren'Py.

use crysirror_python::modules::re as sre;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct ParseError {
    pub msg: String,
    pub filename: Rc<str>,
    pub line: u32,
    pub offset: Option<usize>,
    pub text: Option<String>,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "File \"{}\", line {}: {}", self.filename, self.line, self.msg)?;
        if let Some(t) = &self.text {
            write!(f, "\n    {}", t.lines().next().unwrap_or("").trim())?;
        }
        Ok(())
    }
}

pub type PResult<T> = Result<T, ParseError>;

/// One logical line with its indented sub-block.
#[derive(Clone, Debug)]
pub struct GroupedLine {
    pub filename: Rc<str>,
    pub number: u32,
    pub indent: usize,
    pub text: Rc<[char]>,
    pub block: Rc<Vec<GroupedLine>>,
}

const KEYWORDS: &[&str] = &["as", "if", "in", "return", "with", "while"];
pub const IMAGE_KEYWORDS: &[&str] = &["behind", "at", "onlayer", "with", "zorder", "transform"];

pub const OPERATOR_REGEXP: &str = r"<>|<<|<=|<|>>|>=|>|!=|==|\||\^|&|\+|\-|\*\*|\*|//|/|%|~|@|:=|\bor\b|\band\b|\bnot\b|\bin\b|\bis\b";
pub const WORD_REGEXP: &str = "[a-zA-Z_\u{a0}-\u{fffd}][0-9a-zA-Z_\u{a0}-\u{fffd}]*";
pub const IMAGE_WORD_REGEXP: &str = "[-0-9a-zA-Z_\u{a0}-\u{fffd}][-0-9a-zA-Z_\u{a0}-\u{fffd}]*";

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || (c as u32) >= 128
}

/// File-private name prefix (`munge_filename`).
pub fn munge_prefix(filename: &str) -> String {
    let base = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let base = base.strip_suffix("_ren.py").unwrap_or(base);
    let base = match base.rfind('.') {
        Some(i) if i > 0 => &base[..i],
        _ => base,
    };
    let base = base.replace(' ', "_");
    let mut out = String::from("_m1_");
    for c in base.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push_str(&format!("{:#x}", c as u32));
        }
    }
    out.push_str("__");
    out
}

/// Position after a run of identifier characters starting at `pos`.
pub fn match_logical_word(data: &[char], pos: usize) -> Option<usize> {
    let mut i = pos;
    while i < data.len() && is_ident_char(data[i]) {
        i += 1;
    }
    (i != pos).then_some(i)
}

fn match_operator(d: &[char], pos: usize) -> Option<usize> {
    let c1 = d.get(pos).copied().unwrap_or('\0');
    let c2 = d.get(pos + 1).copied().unwrap_or('\0');
    let c3 = d.get(pos + 2).copied().unwrap_or('\0');
    if (c1 == '.' && c2 == '.' && c3 == '.') || (c3 == '=' && matches!((c1, c2), ('/', '/') | ('>', '>') | ('<', '<') | ('*', '*'))) {
        return Some(pos + 3);
    }
    if matches!((c1, c2), ('/', '/') | ('>', '>') | ('<', '<') | ('<', '>') | ('*', '*') | ('-', '>')) || (c2 == '=' && "+-*/%@&|^:<>=!".contains(c1)) {
        return Some(pos + 2);
    }
    if "+-*/%@&|^,:!.;=~<>$?[]{}()".contains(c1) && c1 != '\0' {
        return Some(pos + 1);
    }
    None
}

pub struct StringMatch {
    pub end: usize,
    pub need_munge: bool,
    pub newlines: usize,
    pub line_start: Option<usize>,
}

/// Scans a Python/Ren'Py string literal (`lexersupport.match_string`).
/// `Err(())` means unterminated.
pub fn match_string(d: &[char], prefix_pos: usize, pos: usize) -> Result<Option<StringMatch>, ()> {
    if pos >= d.len() {
        return Ok(None);
    }
    let mut c = d[pos];
    if !matches!(c, '"' | '\'' | '`') {
        return Ok(None);
    }
    let prefix: String = d[prefix_pos..pos].iter().collect::<String>().to_ascii_lowercase();
    let f_string = match prefix.as_str() {
        "" | "r" | "u" | "b" | "rb" | "br" => false,
        "f" | "rf" | "fr" => true,
        _ => return Ok(None),
    };
    let mut p = pos + 1;
    let quote = c;
    let mut quote_size = 1;
    if p < d.len() && d[p] == quote {
        p += 1;
        if p < d.len() && d[p] == quote {
            quote_size = 3;
            p += 1;
        } else {
            return Ok(Some(StringMatch { end: p, need_munge: false, newlines: 0, line_start: None }));
        }
    }
    let mut newlines = 0;
    let mut line_start = None;
    let mut need_munge = false;
    let mut end_quote = 0;
    let mut last_c;
    while end_quote != quote_size {
        if p >= d.len() {
            return Err(());
        }
        last_c = c;
        c = d[p];
        p += 1;
        if c == '\\' {
            if p >= d.len() {
                return Err(());
            }
            if d[p] == '\n' {
                line_start = Some(p + 1);
                newlines += 1;
            }
            end_quote = 0;
            p += 1;
            continue;
        }
        if f_string && (c == '{' || c == '}') && p < d.len() && d[p] == c {
            end_quote = 0;
            p += 1;
            continue;
        }
        if f_string && c == '{' {
            end_quote = 0;
            let mut depth = 1;
            while depth > 0 {
                if p >= d.len() {
                    return Err(());
                }
                let ch = d[p];
                // Nested string inside the replacement field.
                let mut np = p;
                while np > 0 && p - np < 2 && "rRuUbBfF".contains(d[np - 1]) {
                    np -= 1;
                }
                if np > 0 && is_ident_char(d[np - 1]) {
                    np = p;
                }
                if matches!(ch, '"' | '\'') {
                    match match_string(d, np, p)? {
                        Some(m) => {
                            p = m.end;
                            need_munge |= m.need_munge;
                            newlines += m.newlines;
                            if m.line_start.is_some() {
                                line_start = m.line_start;
                            }
                            continue;
                        }
                        None => {}
                    }
                }
                if ch == '\n' {
                    newlines += 1;
                    line_start = Some(p + 1);
                }
                if ch == '{' {
                    depth += 1;
                } else if ch == '}' {
                    depth -= 1;
                } else if ch == '_' && d.get(p + 1) == Some(&'_') && (p == 0 || !is_ident_char(d[p - 1])) {
                    need_munge = true;
                }
                p += 1;
            }
            c = '}';
            continue;
        }
        let _ = last_c;
        if c == '\n' {
            line_start = Some(p);
            newlines += 1;
        }
        if c == quote {
            end_quote += 1;
            continue;
        }
        end_quote = 0;
        if c == '_' && d.get(p) == Some(&'_') && p >= 2 && matches!(d[p - 2], '.' | '[') {
            need_munge = true;
        }
    }
    Ok(Some(StringMatch { end: p, need_munge, newlines, line_start }))
}

fn munge_names_in(text: &[char], prefix: &str, in_string: bool) -> String {
    // Replaces `__name` (not dunder) with the private prefix. In strings only
    // after `.` or an odd run of `[` (interpolation), as Ren'Py does.
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let c = text[i];
        if c == '_' && text.get(i + 1) == Some(&'_') && (i == 0 || !is_ident_char(text[i - 1])) {
            let mut j = i + 2;
            while j < text.len() && is_ident_char(text[j]) {
                j += 1;
            }
            let name: String = text[i + 2..j].iter().collect();
            let ok_word = !name.is_empty() && !name.ends_with("__") && !name.starts_with('_');
            let ctx_ok = if in_string {
                let mut k = i;
                let mut brackets = 0;
                while k > 0 && text[k - 1] == '[' {
                    brackets += 1;
                    k -= 1;
                }
                (i > 0 && text[i - 1] == '.') || brackets % 2 == 1
            } else {
                true
            };
            if ok_word && ctx_ok && !name.contains("__") {
                out.push_str(prefix);
                out.push_str(&name);
                i = j;
                continue;
            }
            out.extend(&text[i..j]);
            i = j;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// `ren_py_to_rpy(text, filename)`: the `.rpy` text of an `_ren.py` file
/// (7.6/8.1+), keeping line numbers. Only `"""renpy` blocks are Ren'Py
/// script; the Python after a block is indented under its last statement.
pub fn ren_py_to_rpy(text: &str, filename: Option<&str>) -> Result<String, String> {
    #[derive(PartialEq)]
    enum St {
        Ignore,
        Renpy,
        Python,
    }
    let mut lines: Vec<&str> = text.lines().collect();
    let first;
    if let Some(l) = lines.first()
        && let Some(rest) = l.strip_prefix('\u{feff}')
    {
        first = rest.to_owned();
        lines[0] = &first;
    }
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut prefix = String::new();
    let mut state = St::Ignore;
    let mut open_line = 0;
    for (i, l) in lines.iter().enumerate() {
        if state != St::Renpy && l.starts_with("\"\"\"renpy") {
            state = St::Renpy;
            out.push(String::new());
            open_line = i + 1;
            continue;
        }
        match state {
            St::Renpy => {
                if *l == "\"\"\"" {
                    state = St::Python;
                    out.push(String::new());
                    continue;
                }
                let sl = l.trim();
                if sl.is_empty() || sl.starts_with('#') {
                    out.push((*l).to_owned());
                    continue;
                }
                prefix = l.chars().take_while(|c| *c == ' ').collect();
                if sl.ends_with(':') {
                    prefix.push_str("    ");
                }
                out.push((*l).to_owned());
            }
            St::Python => out.push(format!("{prefix}{l}")),
            St::Ignore => out.push(String::new()),
        }
    }
    if let Some(f) = filename {
        if state == St::Ignore {
            return Err(format!("In {f:?}, there are no \"\"\"renpy blocks, so every line is ignored."));
        }
        if state == St::Renpy {
            return Err(format!("In {f:?}, there is a \"\"\"renpy block at line {open_line} that is not terminated by \"\"\"."));
        }
    }
    Ok(out.join("\n"))
}

/// Splits source text into logical lines `(line number, text)`.
pub fn list_logical_lines(filename: &Rc<str>, data: &str, first_line: u32) -> PResult<Vec<(u32, String)>> {
    let mut chars: Vec<char> = data.chars().filter(|c| *c != '\r').collect();
    if chars.first() == Some(&'\u{feff}') {
        chars.remove(0);
    }
    chars.extend(['\n', '\n']);
    let d = &chars[..];
    let prefix = munge_prefix(filename);
    let err = |msg: &str, line: u32| ParseError { msg: msg.into(), filename: filename.clone(), line, offset: None, text: None };
    let mut out = Vec::new();
    let mut pos = 0;
    let mut number = first_line;
    let mut start_number = number;
    let mut line = String::new();
    let mut parens: Vec<(char, u32)> = Vec::new();
    let mut last_append = 0;
    let mut new_line = true;
    while pos < d.len() {
        if new_line {
            // Skip blank/comment-only lines (IGNORE_PATTERN).
            loop {
                let mut p = pos;
                while p < d.len() && d[p] == ' ' {
                    p += 1;
                }
                if p < d.len() && d[p] == '#' {
                    while p < d.len() && d[p] != '\n' {
                        p += 1;
                    }
                }
                if p < d.len() && d[p] == '\\' && d.get(p + 1) == Some(&'\n') {
                    p += 1;
                }
                if p < d.len() && d[p] == '\n' {
                    pos = p + 1;
                    number += 1;
                    continue;
                }
                break;
            }
            if pos >= d.len() {
                break;
            }
            start_number = number;
            last_append = pos;
            line.clear();
            new_line = false;
            continue;
        }
        while pos < d.len() && d[pos] == ' ' {
            pos += 1;
        }
        let word = match_logical_word(d, pos);
        match match_string(d, pos, word.unwrap_or(pos)) {
            Err(()) => return Err(err("unterminated string literal", start_number)),
            Ok(Some(m)) => {
                number += m.newlines as u32;
                if m.need_munge {
                    line.extend(&d[last_append..pos]);
                    line.push_str(&munge_names_in(&d[pos..m.end], &prefix, true));
                    last_append = m.end;
                }
                pos = m.end;
                continue;
            }
            Ok(None) => {}
        }
        if let Some(wp) = word {
            if d[pos] == '_' && d.get(pos + 1) == Some(&'_') {
                let w: String = d[pos..wp].iter().collect();
                let munged = munge_names_in(&d[pos..wp], &prefix, false);
                if munged != w {
                    line.extend(&d[last_append..pos]);
                    line.push_str(&munged);
                    last_append = wp;
                }
            }
            pos = wp;
            continue;
        }
        let mut c = d[pos];
        if c == '#' {
            line.extend(&d[last_append..pos]);
            while pos < d.len() && d[pos] != '\n' {
                pos += 1;
            }
            last_append = pos;
            c = '\n';
        }
        if c == '\n' {
            if !parens.is_empty() {
                pos += 1;
                number += 1;
                continue;
            }
            if last_append != pos {
                line.extend(&d[last_append..pos]);
            }
            if !line.trim().is_empty() {
                out.push((start_number, std::mem::take(&mut line)));
            }
            new_line = true;
            pos += 1;
            number += 1;
            continue;
        }
        if "([{".contains(c) {
            parens.push((c, number));
            pos += 1;
            continue;
        }
        if ")]}".contains(c) {
            let Some((open, _)) = parens.pop() else {
                return Err(err(&format!("unmatched '{c}'"), number));
            };
            if !matches!((open, c), ('(', ')') | ('[', ']') | ('{', '}')) {
                return Err(err(&format!("closing parenthesis '{c}' does not match opening parenthesis '{open}'"), number));
            }
            pos += 1;
            continue;
        }
        if let Some(op) = match_operator(d, pos) {
            pos = op;
            continue;
        }
        if c == '\\' {
            if d.get(pos + 1) != Some(&'\n') {
                return Err(err("unexpected character after line continuation character.", number));
            }
            number += 1;
            pos += 2;
            continue;
        }
        if c == '\t' {
            return Err(err("Tab character is not allowed in Ren'Py scripts.", number));
        }
        // Other characters (e.g. '"' handled above) are passed through.
        pos += 1;
    }
    if let Some((c, l)) = parens.last() {
        return Err(err(&format!("'{c}' was never closed"), *l));
    }
    Ok(out)
}

/// Groups logical lines into indentation blocks.
pub fn group_logical_lines(filename: &Rc<str>, lines: Vec<(u32, String)>) -> PResult<Vec<GroupedLine>> {
    struct Frame {
        indent: usize,
        lines: Vec<GroupedLine>,
    }
    fn close(f: Frame, parent: &mut Frame) {
        if let Some(last) = parent.lines.last_mut() {
            last.block = Rc::new(f.lines);
        }
    }
    if let Some((n, t)) = lines.first() {
        if t.starts_with(' ') {
            return Err(ParseError { msg: "Unexpected indentation at start of file.".into(), filename: filename.clone(), line: *n, offset: None, text: Some(t.clone()) });
        }
    }
    let mut stack = vec![Frame { indent: 0, lines: Vec::new() }];
    for (number, text) in lines {
        let rest = text.trim_start_matches(' ');
        let indent = text.len() - rest.len();
        let cur = stack.last().unwrap().indent;
        if indent > cur {
            stack.push(Frame { indent, lines: Vec::new() });
        } else if indent < cur {
            while stack.last().unwrap().indent > indent {
                let f = stack.pop().unwrap();
                close(f, stack.last_mut().unwrap());
            }
            if stack.last().unwrap().indent != indent {
                return Err(ParseError { msg: "unindent does not match any outer indentation level".into(), filename: filename.clone(), line: number, offset: Some(indent + 1), text: Some(rest.into()) });
            }
        }
        let gl = GroupedLine { filename: filename.clone(), number, indent, text: rest.trim_end().chars().collect::<Vec<_>>().into(), block: Rc::new(Vec::new()) };
        stack.last_mut().unwrap().lines.push(gl);
    }
    while stack.len() > 1 {
        let f = stack.pop().unwrap();
        close(f, stack.last_mut().unwrap());
    }
    Ok(stack.pop().unwrap().lines)
}

/// A Python expression or statement text with its source location.
#[derive(Clone, Debug, PartialEq)]
pub struct PyExpr {
    pub text: Rc<str>,
    pub filename: Rc<str>,
    pub line: u32,
}

impl PyExpr {
    pub fn new(text: impl Into<Rc<str>>, filename: Rc<str>, line: u32) -> PyExpr {
        PyExpr { text: text.into(), filename, line }
    }
}

thread_local! {
    static RE_CACHE: std::cell::RefCell<HashMap<String, Rc<sre::Program>>> = std::cell::RefCell::new(HashMap::new());
}

fn program(pat: &str) -> Rc<sre::Program> {
    RE_CACHE.with(|c| {
        if let Some(p) = c.borrow().get(pat) {
            return p.clone();
        }
        let p = Rc::new(sre::compile(pat, sre::S | sre::U).unwrap_or_else(|e| panic!("bad lexer regexp {pat}: {e}")));
        c.borrow_mut().insert(pat.to_string(), p.clone());
        p
    })
}

/// Fallible variant for patterns supplied by scripts.
pub fn try_program(pat: &str) -> Result<Rc<sre::Program>, String> {
    RE_CACHE.with(|c| {
        if let Some(p) = c.borrow().get(pat) {
            return Ok(p.clone());
        }
        let p = Rc::new(sre::compile(pat, sre::S | sre::U)?);
        c.borrow_mut().insert(pat.to_string(), p.clone());
        Ok(p)
    })
}

#[derive(Clone)]
pub struct Checkpoint {
    line: isize,
    pos: usize,
}

#[derive(Clone)]
pub struct Lexer {
    pub block: Rc<Vec<GroupedLine>>,
    pub init: bool,
    pub init_offset: i32,
    pub eob: bool,
    pub line: isize,
    pub filename: Rc<str>,
    pub text: Rc<[char]>,
    pub number: u32,
    pub column: usize,
    pub subblock: Rc<Vec<GroupedLine>>,
    pub global_label: Option<Rc<str>>,
    pub pos: usize,
    pub monologue_delimiter: String,
    pub strict_expression: bool,
}

impl Lexer {
    pub fn new(block: Rc<Vec<GroupedLine>>, init: bool) -> Lexer {
        let filename = block.first().map(|l| l.filename.clone()).unwrap_or_else(|| "<string>".into());
        Lexer {
            block,
            init,
            init_offset: 0,
            eob: false,
            line: -1,
            filename,
            text: Rc::from(Vec::new()),
            number: 0,
            column: 0,
            subblock: Rc::new(Vec::new()),
            global_label: None,
            pos: 0,
            monologue_delimiter: "\n\n".into(),
            strict_expression: true,
        }
    }

    fn load(&mut self) {
        let l = &self.block[self.line as usize];
        self.filename = l.filename.clone();
        self.number = l.number;
        self.column = l.indent;
        self.text = l.text.clone();
        self.subblock = l.block.clone();
    }

    pub fn advance(&mut self) -> bool {
        self.line += 1;
        if self.line as usize >= self.block.len() {
            self.eob = true;
            return false;
        }
        self.load();
        self.pos = 0;
        true
    }

    pub fn unadvance(&mut self) {
        self.line -= 1;
        self.eob = false;
        if self.line >= 0 {
            self.load();
        }
        self.pos = self.text.len();
    }

    pub fn error<T>(&self, msg: impl Into<String>) -> PResult<T> {
        let (filename, number, text) = if self.line == -1 && !self.block.is_empty() {
            (self.block[0].filename.clone(), self.block[0].number, self.block[0].text.iter().collect())
        } else {
            (self.filename.clone(), self.number, self.text.iter().collect::<String>())
        };
        Err(ParseError { msg: msg.into(), filename, line: number, offset: Some(self.pos + 1), text: Some(text) })
    }

    pub fn skip_whitespace(&mut self) {
        while self.pos < self.text.len() {
            match self.text[self.pos] {
                ' ' | '\n' => self.pos += 1,
                '\\' if self.text.get(self.pos + 1) == Some(&'\n') => self.pos += 2,
                _ => break,
            }
        }
    }

    /// Matches `pat` at the current position without skipping whitespace.
    pub fn match_regexp(&mut self, pat: &str) -> Option<String> {
        if self.eob || self.pos >= self.text.len() {
            return None;
        }
        let prog = program(pat);
        self.match_program(&prog)
    }

    pub fn match_program(&mut self, prog: &sre::Program) -> Option<String> {
        if self.eob || self.pos >= self.text.len() {
            return None;
        }
        let mut m = sre::Matcher::new(prog, &self.text);
        let slots = m.match_at(self.pos, self.text.len(), false)?;
        let end = slots[1]?;
        let s: String = self.text[self.pos..end].iter().collect();
        self.pos = end;
        Some(s)
    }

    pub fn match_re(&mut self, pat: &str) -> Option<String> {
        self.skip_whitespace();
        self.match_regexp(pat)
    }

    pub fn keyword(&mut self, w: &str) -> bool {
        let old = self.pos;
        if self.word().as_deref() == Some(w) {
            return true;
        }
        self.pos = old;
        false
    }

    pub fn eol(&mut self) -> bool {
        self.skip_whitespace();
        self.pos >= self.text.len()
    }

    pub fn expect_eol(&mut self) -> PResult<()> {
        if !self.eol() {
            return self.error("end of line expected.");
        }
        Ok(())
    }

    pub fn expect_noblock(&self, stmt: &str) -> PResult<()> {
        if !self.subblock.is_empty() {
            let mut ll = self.subblock_lexer(false);
            ll.advance();
            return ll.error(format!("Line is indented, but the preceding {stmt} statement does not expect a block. Please check this line's indentation. You may have forgotten a colon (:)."));
        }
        Ok(())
    }

    pub fn expect_block(&self, stmt: &str) -> PResult<()> {
        if self.subblock.is_empty() {
            return self.error(format!("{stmt} expects a non-empty block."));
        }
        Ok(())
    }

    pub fn has_block(&self) -> bool {
        !self.subblock.is_empty()
    }

    pub fn subblock_lexer(&self, init: bool) -> Lexer {
        let mut l = Lexer::new(self.subblock.clone(), self.init || init);
        l.init_offset = self.init_offset;
        l.global_label = self.global_label.clone();
        l.monologue_delimiter = self.monologue_delimiter.clone();
        l.strict_expression = self.strict_expression;
        l.filename = self.filename.clone();
        l
    }

    fn dequote(s: &str) -> String {
        let chars: Vec<char> = s.chars().collect();
        let mut out = String::with_capacity(s.len());
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() {
                let c = chars[i + 1];
                i += 2;
                match c {
                    '{' => out.push_str("{{"),
                    '[' => out.push_str("[["),
                    '%' => out.push_str("%%"),
                    'n' => out.push('\n'),
                    'u' => {
                        let mut hex = String::new();
                        while hex.len() < 4 && i < chars.len() && chars[i].is_ascii_hexdigit() {
                            hex.push(chars[i]);
                            i += 1;
                        }
                        if !hex.is_empty() {
                            if let Some(ch) = char::from_u32(u32::from_str_radix(&hex, 16).unwrap_or(0xfffd)) {
                                out.push(ch);
                            }
                        }
                    }
                    other => out.push(other),
                }
                continue;
            }
            out.push(chars[i]);
            i += 1;
        }
        out
    }

    fn collapse(s: &str, re_ws: &str) -> String {
        // re.sub(re_ws, " ") for the two patterns Ren'Py uses.
        let mut out = String::with_capacity(s.len());
        let mut in_ws = false;
        for c in s.chars() {
            let ws = match re_ws {
                "space_nl" => c == ' ' || c == '\n',
                _ => c == ' ',
            };
            if ws {
                if !in_ws {
                    out.push(' ');
                }
                in_ws = true;
            } else {
                out.push(c);
                in_ws = false;
            }
        }
        out
    }

    /// A single-quoted Ren'Py string (dialogue form, whitespace collapsed).
    pub fn string(&mut self) -> Option<String> {
        let s = self.match_re(r#"r?"([^\\"]|\\.)*""#).or_else(|| self.match_re(r"r?'([^\\']|\\.)*'")).or_else(|| self.match_re(r"r?`([^\\`]|\\.)*`"))?;
        let (raw, s) = match s.strip_prefix('r') {
            Some(r) => (true, r.to_string()),
            None => (false, s),
        };
        let inner: String = s.chars().skip(1).take(s.chars().count() - 2).collect();
        if raw {
            return Some(inner);
        }
        Some(Self::dequote(&Self::collapse(&inner, "space_nl")))
    }

    /// Triple-quoted string: returns either a raw string or monologue parts.
    pub fn triple_string(&mut self) -> Option<TripleString> {
        let s = self.match_re(r#"r?"""([^\\"]|\\.|"(?!""))*""""#).or_else(|| self.match_re(r"r?'''([^\\']|\\.|'(?!''))*'''")).or_else(|| self.match_re(r"r?```([^\\`]|\\.|`(?!``))*```"))?;
        let (raw, s) = match s.strip_prefix('r') {
            Some(r) => (true, r.to_string()),
            None => (false, s),
        };
        let n = s.chars().count();
        let inner: String = s.chars().skip(3).take(n - 6).collect();
        if raw {
            return Some(TripleString::Raw(inner));
        }
        // re.sub(" *\n *", "\n", s)
        let mut norm = String::new();
        for (i, l) in inner.split('\n').enumerate() {
            if i > 0 {
                norm.push('\n');
            }
            let l = if i > 0 { l.trim_start_matches(' ') } else { l };
            norm.push_str(l.trim_end_matches(' '));
        }
        let mondel = self.monologue_delimiter.clone();
        let parts: Vec<&str> = if mondel.is_empty() { vec![&norm[..]] } else { norm.split(mondel.as_str()).collect() };
        let mut rv = Vec::new();
        let mut pos = 0;
        for part in parts {
            let leading = part.len() - part.trim_start().len();
            let line_offset = norm[..pos.min(norm.len())].matches('\n').count() + part[..leading].matches('\n').count();
            pos += part.len() + mondel.len();
            let p = part.trim();
            if p.is_empty() {
                continue;
            }
            let p = if mondel.is_empty() { Self::collapse(p, "space") } else { Self::collapse(p, "space_nl") };
            rv.push((line_offset as u32, Self::dequote(&p)));
        }
        Some(TripleString::Parts(rv))
    }

    pub fn integer(&mut self) -> Option<String> {
        self.match_re(r"(\+|\-)?\d+")
    }

    pub fn float(&mut self) -> Option<String> {
        self.match_re(r"(\+|\-)?(\d+\.?\d*|\.\d+)([eE][-+]?\d+)?")
    }

    pub fn hash(&mut self) -> Option<String> {
        self.match_re(r"\w+")
    }

    pub fn word(&mut self) -> Option<String> {
        self.skip_whitespace();
        let np = match_logical_word(&self.text, self.pos)?;
        let w: String = self.text[self.pos..np].iter().collect();
        let mut it = w.chars();
        let first = it.next()?;
        if !(first == '_' || first.is_alphabetic()) || !it.all(|c| c == '_' || c.is_alphanumeric()) {
            return None;
        }
        self.pos = np;
        Some(w)
    }

    fn is_string_start(&self) -> bool {
        matches!(self.text.get(self.pos), Some('"') | Some('\'') | Some('`'))
    }

    pub fn name(&mut self) -> Option<String> {
        let old = self.pos;
        let w = self.word()?;
        if matches!(w.as_str(), "r" | "u" | "ur") && self.is_string_start() {
            self.pos = old;
            return None;
        }
        if KEYWORDS.contains(&w.as_str()) {
            self.pos = old;
            return None;
        }
        Some(w)
    }

    pub fn set_global_label(&mut self, label: &str) {
        if !label.is_empty() && !label.starts_with('.') {
            self.global_label = Some(label.split('.').next().unwrap().into());
        }
    }

    pub fn label_name(&mut self) -> Option<String> {
        let old = self.pos;
        let global = self.name();
        let (g, local) = match global {
            None => {
                if self.match_re(r"\.").is_none() || self.global_label.is_none() {
                    self.pos = old;
                    return None;
                }
                let Some(l) = self.name() else {
                    self.pos = old;
                    return None;
                };
                (self.global_label.as_deref().unwrap().to_string(), Some(l))
            }
            Some(g) => {
                if self.match_re(r"\.").is_some() {
                    let Some(l) = self.name() else {
                        self.pos = old;
                        return None;
                    };
                    (g, Some(l))
                } else {
                    (g, None)
                }
            }
        };
        Some(match local {
            Some(l) => format!("{g}.{l}"),
            None => g,
        })
    }

    pub fn image_name_component(&mut self) -> Option<String> {
        let old = self.pos;
        let rv = self.match_re(IMAGE_WORD_REGEXP)?;
        if matches!(rv.as_str(), "r" | "u" | "ur") && self.is_string_start() {
            self.pos = old;
            return None;
        }
        if KEYWORDS.contains(&rv.as_str()) || IMAGE_KEYWORDS.contains(&rv.as_str()) {
            self.pos = old;
            return None;
        }
        Some(rv)
    }

    pub fn python_string(&mut self) -> PResult<bool> {
        if self.eol() {
            return Ok(false);
        }
        let start = self.pos;
        let delim = match_logical_word(&self.text, start);
        let at = delim.unwrap_or(start);
        if at >= self.text.len() || !matches!(self.text[at], '"' | '\'') {
            return Ok(false);
        }
        match match_string(&self.text, start, at) {
            Err(()) => self.error("end of line reached while parsing string."),
            Ok(Some(m)) => {
                self.pos = m.end;
                Ok(true)
            }
            Ok(None) => Ok(false),
        }
    }

    pub fn dotted_name(&mut self) -> PResult<Option<String>> {
        let Some(mut rv) = self.name() else { return Ok(None) };
        while self.match_re(r"\.").is_some() {
            match self.name() {
                Some(n) => {
                    rv.push('.');
                    rv.push_str(&n);
                }
                None => return self.error("expecting name."),
            }
        }
        Ok(Some(rv))
    }

    fn slice(&self, a: usize, b: usize) -> String {
        self.text[a..b].iter().collect()
    }

    fn strip_eval(&self, start: usize, end: usize) -> String {
        let s = self.slice(start, end);
        let mut s = s.trim().to_string();
        while s.ends_with('\\') {
            s.pop();
            s = s.trim_end().to_string();
        }
        s
    }

    pub fn expr(&self, s: String) -> PyExpr {
        PyExpr::new(s, self.filename.clone(), self.number)
    }

    pub fn delimited_python(&mut self, delim: &str) -> PResult<String> {
        let start = self.pos;
        while !self.eol() {
            let c = self.text[self.pos];
            if delim.contains(c) {
                return Ok(self.slice(start, self.pos));
            }
            if self.python_string()? {
                continue;
            }
            if self.parenthesised_python()? {
                continue;
            }
            self.pos += 1;
        }
        self.error(format!("reached end of line when expecting '{delim}'."))
    }

    pub fn python_expression(&mut self) -> PResult<PyExpr> {
        let start = self.pos;
        let pe = self.delimited_python(":")?;
        if pe.trim().is_empty() {
            return self.error("expected python_expression");
        }
        Ok(self.expr(self.strip_eval(start, self.pos)))
    }

    pub fn parenthesised_python(&mut self) -> PResult<bool> {
        let Some(&c) = self.text.get(self.pos) else { return Ok(false) };
        let close = match c {
            '(' => ")",
            '[' => "]",
            '{' => "}",
            _ => return Ok(false),
        };
        self.pos += 1;
        self.delimited_python(close)?;
        self.pos += 1;
        Ok(true)
    }

    pub fn simple_expression_text(&mut self, comma: bool, operator: bool, image: bool) -> PResult<Option<String>> {
        let start = self.pos;
        loop {
            while self.match_re(OPERATOR_REGEXP).is_some() {}
            if self.eol() {
                break;
            }
            let ok = self.python_string()? || {
                let old = self.pos;
                match self.name() {
                    Some(n) if image && IMAGE_KEYWORDS.contains(&n.as_str()) => {
                        self.pos = old;
                        false
                    }
                    Some(_) => true,
                    None => false,
                }
            } || self.float().is_some()
                || self.parenthesised_python()?;
            if !ok {
                break;
            }
            loop {
                self.skip_whitespace();
                if self.eol() {
                    break;
                }
                if self.match_re(r"\.").is_some() {
                    if self.word().is_none() {
                        return self.error("expecting name after dot.");
                    }
                    continue;
                }
                if self.parenthesised_python()? {
                    continue;
                }
                break;
            }
            if operator && self.match_re(OPERATOR_REGEXP).is_some() {
                continue;
            }
            if comma && self.match_re(",").is_some() {
                continue;
            }
            break;
        }
        let s = self.strip_eval(start, self.pos);
        Ok(if s.is_empty() { None } else { Some(s) })
    }

    pub fn simple_expression(&mut self) -> PResult<Option<PyExpr>> {
        Ok(self.simple_expression_text(false, true, false)?.map(|s| self.expr(s)))
    }

    pub fn image_simple_expression(&mut self) -> PResult<Option<PyExpr>> {
        Ok(self.simple_expression_text(false, true, true)?.map(|s| self.expr(s)))
    }

    pub fn comma_expression(&mut self) -> PResult<Option<PyExpr>> {
        Ok(self.simple_expression_text(true, true, false)?.map(|s| self.expr(s)))
    }

    pub fn say_expression(&mut self) -> PResult<Option<PyExpr>> {
        Ok(self.simple_expression_text(false, false, false)?.map(|s| self.expr(s)))
    }

    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint { line: self.line, pos: self.pos }
    }

    pub fn revert(&mut self, c: &Checkpoint) {
        self.line = c.line;
        if self.line >= 0 && (self.line as usize) < self.block.len() {
            self.load();
            self.eob = false;
        } else {
            self.eob = self.line >= 0;
        }
        self.pos = c.pos;
    }

    pub fn require_re(&mut self, pat: &str, name: &str) -> PResult<String> {
        match self.match_re(pat) {
            Some(s) => Ok(s),
            None => self.error(format!("expected '{name}' not found.")),
        }
    }

    pub fn require_name(&mut self) -> PResult<String> {
        match self.name() {
            Some(s) => Ok(s),
            None => self.error("expected 'name' not found."),
        }
    }

    pub fn require_simple(&mut self) -> PResult<PyExpr> {
        match self.simple_expression()? {
            Some(s) => Ok(s),
            None => self.error("expected 'simple_expression' not found."),
        }
    }

    pub fn rest(&mut self) -> PyExpr {
        self.skip_whitespace();
        let s = self.slice(self.pos, self.text.len());
        self.pos = self.text.len();
        self.expr(s.trim().to_string())
    }

    pub fn raw_rest(&mut self) -> String {
        self.skip_whitespace();
        let s = self.slice(self.pos, self.text.len());
        self.pos = self.text.len();
        s
    }

    /// The sub-block as Python source, padded so line numbers match.
    pub fn python_block(&self) -> String {
        fn walk(block: &[GroupedLine], base_indent: usize, out: &mut String, line: &mut u32) {
            for l in block {
                let prefix = " ".repeat(l.indent.saturating_sub(base_indent));
                while *line < l.number {
                    out.push_str(&prefix);
                    out.push('\n');
                    *line += 1;
                }
                let t: String = l.text.iter().collect();
                out.push_str(&prefix);
                out.push_str(&t);
                out.push('\n');
                *line += 1 + t.matches('\n').count() as u32;
                walk(&l.block, base_indent, out, line);
            }
        }
        let mut out = String::new();
        let mut line = self.number + 1;
        if let Some(first) = self.subblock.first() {
            walk(&self.subblock, first.indent, &mut out, &mut line);
        }
        out
    }

    pub fn location(&self) -> (Rc<str>, u32) {
        (self.filename.clone(), self.number)
    }

    pub fn current_text(&self) -> String {
        self.text.iter().collect()
    }
}

pub enum TripleString {
    Raw(String),
    Parts(Vec<(u32, String)>),
}

/// Lexes a whole file into grouped lines.
pub fn lex_file(filename: &str, data: &str) -> PResult<Rc<Vec<GroupedLine>>> {
    let fname: Rc<str> = filename.into();
    let lines = list_logical_lines(&fname, data, 1)?;
    Ok(Rc::new(group_logical_lines(&fname, lines)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logical_lines() {
        let src = "label start:\n    e \"Hello, (world)\" # comment\n    $ x = (1,\n        2)\n\n    return\n";
        let b = lex_file("game/script.rpy", src).unwrap();
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].block.len(), 3);
        assert_eq!(b[0].block[1].text.iter().collect::<String>(), "$ x = (1,\n        2)");
        let mut l = Lexer::new(b.clone(), false);
        l.advance();
        assert!(l.keyword("label"));
        assert_eq!(l.label_name().as_deref(), Some("start"));
        assert!(l.match_re(":").is_some());
        assert!(l.eol());
        let mut s = l.subblock_lexer(false);
        s.advance();
        assert_eq!(s.say_expression().unwrap().unwrap().text.as_ref(), "e");
        assert_eq!(s.string().as_deref(), Some("Hello, (world)"));
    }
    #[test]
    fn munging() {
        assert_eq!(munge_prefix("game/script.rpy"), "_m1_script__");
        let b = lex_file("game/a b.rpy", "$ __x = 1\n").unwrap();
        assert_eq!(b[0].text.iter().collect::<String>(), "$ _m1_a_b__x = 1");
    }
}

fn is_word_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || ('\u{a0}'..='\u{fffd}').contains(&c)
}

fn is_word_char(c: char) -> bool {
    is_word_start(c) || c.is_ascii_digit()
}

/// `re.match(word_regexp + r"\s*$", s)`: a bare name, as `Say.who_fast`.
pub fn is_name(s: &str) -> bool {
    let t = s.trim_end();
    let mut cs = t.chars();
    cs.next().is_some_and(is_word_start) && cs.all(is_word_char)
}

/// `renpy.translation.encode_say_string`.
pub fn encode_say_string(s: &str) -> String {
    let s = s.replace('\\', "\\\\").replace('\n', "\\n").replace('"', "\\\"");
    // `re.sub(r'(?<= ) ', '\\ ', s)`: a space preceded by a space.
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let mut prev_space = false;
    for c in s.chars() {
        if c == ' ' && prev_space {
            out.push_str("\\ ");
        } else {
            out.push(c);
        }
        prev_space = c == ' ';
    }
    out.push('"');
    out
}
