// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Bounded, VM-independent document parsing and writing.
//!
//! The `JsonDialect::KiriKiri` profile follows the parameter-independent JSON
//! grammar documented by wamsoft/json 331577f704d1d64c648646f2a4fdfd4b816f9387.
//! No upstream parser, VM, native filesystem or serializer code is imported.
use anyhow::{bail, ensure, Result};
/// A document value with exact signed integers, explicit void and ordered keys.
#[derive(Clone, Debug, PartialEq)]
pub enum DocumentValue {
    Void,
    Integer(i64),
    Real(f64),
    Text(String),
    Array(Vec<DocumentValue>),
    Object(Vec<(String, DocumentValue)>),
}
/// Explicit grammar; KiriKiri accepts comments, quoted keys, single quotes,
/// alternative separators, void and empty array slots. It retains strtoll's
/// historical `1e5 -> 1` rule. Bare-word keys are rejected by the reference.
#[derive(Clone, Copy, Debug)]
pub enum JsonDialect {
    KiriKiri,
}
pub const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
pub const MAX_DOCUMENT_NODES: usize = 65536;
pub const MAX_DOCUMENT_DEPTH: usize = 64;
/// TOML 1.1 (toml_edit) into the shared document model. Tables keep their key
/// order; booleans become 0/1 integers and date-times their TOML text, since
/// the model has no separate kinds for them.
pub fn parse_toml(text: &str) -> Result<DocumentValue> {
    ensure!(text.len() <= MAX_DOCUMENT_BYTES, "document byte budget");
    let doc: toml_edit::Document<String> = text.parse().map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut nodes = 0;
    toml_table(doc.as_table(), 0, &mut nodes)
}
fn toml_count(depth: usize, nodes: &mut usize) -> Result<()> {
    *nodes += 1;
    ensure!(*nodes <= MAX_DOCUMENT_NODES && depth <= MAX_DOCUMENT_DEPTH, "document shape budget");
    Ok(())
}
fn toml_table(t: &toml_edit::Table, depth: usize, nodes: &mut usize) -> Result<DocumentValue> {
    toml_count(depth, nodes)?;
    let mut out = Vec::new();
    for (k, item) in t.iter() {
        let v = match item {
            toml_edit::Item::None => continue,
            toml_edit::Item::Value(v) => toml_value(v, depth + 1, nodes)?,
            toml_edit::Item::Table(t) => toml_table(t, depth + 1, nodes)?,
            toml_edit::Item::ArrayOfTables(a) => {
                toml_count(depth + 1, nodes)?;
                DocumentValue::Array(a.iter().map(|t| toml_table(t, depth + 2, nodes)).collect::<Result<_>>()?)
            }
        };
        out.push((k.to_owned(), v));
    }
    Ok(DocumentValue::Object(out))
}
fn toml_value(v: &toml_edit::Value, depth: usize, nodes: &mut usize) -> Result<DocumentValue> {
    use toml_edit::Value as V;
    toml_count(depth, nodes)?;
    Ok(match v {
        V::String(s) => DocumentValue::Text(s.value().clone()),
        V::Integer(i) => DocumentValue::Integer(*i.value()),
        V::Float(f) => DocumentValue::Real(*f.value()),
        V::Boolean(b) => DocumentValue::Integer(i64::from(*b.value())),
        V::Datetime(d) => DocumentValue::Text(d.value().to_string()),
        V::Array(a) => DocumentValue::Array(a.iter().map(|v| toml_value(v, depth + 1, nodes)).collect::<Result<_>>()?),
        V::InlineTable(t) => {
            let mut out = Vec::new();
            for (k, v) in t.iter() {
                out.push((k.to_owned(), toml_value(v, depth + 1, nodes)?));
            }
            DocumentValue::Object(out)
        }
    })
}
/// Pure CPU parser: callers choose a task/owner boundary; no ambient I/O.
pub fn parse_json(text: &str, _dialect: JsonDialect) -> Result<DocumentValue> {
    ensure!(text.len() <= MAX_DOCUMENT_BYTES, "document byte budget");
    let mut p = Parser {
        chars: text.chars().collect(),
        at: 0,
        nodes: 0,
    };
    // Original eval consumes a single value and ignores its trailing input.
    p.value(0)
}
struct Parser {
    chars: Vec<char>,
    at: usize,
    nodes: usize,
}
impl Parser {
    fn get(&mut self) -> Option<char> {
        let c = self.chars.get(self.at).copied();
        self.at += usize::from(c.is_some());
        c
    }
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }
    fn next(&mut self) -> Result<Option<char>> {
        loop {
            match self.get() {
                Some('#') => while !matches!(self.get(), None | Some('\r' | '\n')) {},
                Some('/') if self.peek() == Some('/') => {
                    while !matches!(self.get(), None | Some('\r' | '\n')) {}
                }
                Some('/') if self.peek() == Some('*') => {
                    self.get();
                    loop {
                        match self.get() {
                            None => bail!("unterminated document comment"),
                            Some('*') if self.peek() == Some('/') => {
                                self.get();
                                break;
                            }
                            _ => (),
                        }
                    }
                }
                Some(c) if c == '\u{feff}' || c <= ' ' => (),
                c => return Ok(c),
            }
        }
    }
    fn value(&mut self, depth: usize) -> Result<DocumentValue> {
        self.nodes += 1;
        ensure!(
            self.nodes <= MAX_DOCUMENT_NODES && depth <= MAX_DOCUMENT_DEPTH,
            "document shape budget"
        );
        Ok(match self.next()? {
            Some(q @ ('\'' | '"')) => DocumentValue::Text(self.quoted(q)?),
            Some('[') => self.array(depth + 1)?,
            Some('{') => self.object(depth + 1)?,
            Some(c) if number_char(c) && !matches!(c, 'e' | 'E') => self.number(c)?,
            Some(c) if c.is_ascii_lowercase() => {
                let mut s = String::from(c);
                while self.peek().is_some_and(|c| c.is_ascii_lowercase()) {
                    s.push(self.get().unwrap())
                }
                match s.as_str() {
                    "true" => DocumentValue::Integer(1),
                    "false" => DocumentValue::Integer(0),
                    "null" | "void" => DocumentValue::Void,
                    _ => bail!("unknown document keyword"),
                }
            }
            _ => bail!("document value expected"),
        })
    }
    fn array(&mut self, depth: usize) -> Result<DocumentValue> {
        let mut out = vec![];
        loop {
            match self.next()? {
                Some(']') => break,
                Some(',' | ';') => {
                    self.at -= 1;
                    self.nodes += 1;
                    ensure!(self.nodes <= MAX_DOCUMENT_NODES, "document node budget");
                    out.push(DocumentValue::Void)
                }
                Some(_) => {
                    self.at -= 1;
                    out.push(self.value(depth)?)
                }
                None => bail!("unterminated document array"),
            }
            match self.next()? {
                Some(',' | ';') => (),
                Some(']') => break,
                _ => bail!("document array separator expected"),
            }
        }
        Ok(DocumentValue::Array(out))
    }
    fn object(&mut self, depth: usize) -> Result<DocumentValue> {
        let mut out: Vec<(String, DocumentValue)> = vec![];
        let mut positions = std::collections::BTreeMap::<String, usize>::new();
        loop {
            match self.next()? {
                Some('}') => break,
                Some(',' | ';') => {
                    self.at -= 1;
                }
                Some(_) => {
                    self.at -= 1;
                    let key = match self.value(depth)? {
                        DocumentValue::Text(s) => s,
                        DocumentValue::Integer(i) => i.to_string(),
                        DocumentValue::Real(f) => f.to_string(),
                        DocumentValue::Void => String::new(),
                        _ => bail!("scalar document key required"),
                    };
                    match self.next()? {
                        Some(':') => (),
                        Some('=') => {
                            if self.peek() == Some('>') {
                                self.get();
                            }
                        }
                        _ => bail!("document key separator expected"),
                    }
                    let value = self.value(depth)?;
                    if let Some(&i) = positions.get(&key) {
                        out[i].1 = value
                    } else {
                        positions.insert(key.clone(), out.len());
                        out.push((key, value))
                    }
                }
                None => bail!("unterminated document object"),
            }
            match self.next()? {
                Some(',' | ';') => (),
                Some('}') => break,
                _ => bail!("document object separator expected"),
            }
        }
        Ok(DocumentValue::Object(out))
    }
    fn quoted(&mut self, q: char) -> Result<String> {
        let mut out = String::new();
        loop {
            match self.get() {
                Some(c) if c == q => return Ok(out),
                None | Some('\0' | '\r' | '\n') => bail!("unterminated document string"),
                Some('\\') => {
                    let c = self
                        .get()
                        .ok_or_else(|| anyhow::anyhow!("truncated escape"))?;
                    let decoded = match c {
                        'b' => '\u{8}',
                        'f' => '\u{c}',
                        't' => '\t',
                        'r' => '\r',
                        'n' => '\n',
                        'u' | 'x' => {
                            let n = if c == 'u' { 4 } else { 2 };
                            let mut value = 0u32;
                            for _ in 0..n {
                                value = value * 16
                                    + self.get().and_then(|c| c.to_digit(16)).ok_or_else(|| {
                                        anyhow::anyhow!("invalid document hex escape")
                                    })?
                            }
                            if (0xd800..=0xdbff).contains(&value) {
                                ensure!(
                                    self.get() == Some('\\') && self.get() == Some('u'),
                                    "document surrogate pair required"
                                );
                                let mut low = 0u32;
                                for _ in 0..4 {
                                    low = low * 16
                                        + self.get().and_then(|c| c.to_digit(16)).ok_or_else(
                                            || anyhow::anyhow!("invalid document surrogate"),
                                        )?
                                }
                                ensure!(
                                    (0xdc00..=0xdfff).contains(&low),
                                    "invalid document surrogate"
                                );
                                value = 0x10000 + ((value - 0xd800) << 10) + (low - 0xdc00);
                            }
                            char::from_u32(value)
                                .ok_or_else(|| anyhow::anyhow!("invalid document Unicode scalar"))?
                        }
                        other => other,
                    };
                    out.push(decoded)
                }
                Some(c) => out.push(c),
            }
        }
    }
    fn number(&mut self, first: char) -> Result<DocumentValue> {
        let mut token = String::from(first);
        while self.peek().is_some_and(number_char) {
            token.push(self.get().unwrap())
        }
        if token.contains('.') {
            let raw = token.as_bytes();
            let mut i = usize::from(matches!(raw.first(), Some(b'+' | b'-')));
            let mut digits = 0;
            while raw.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
                digits += 1
            }
            if raw.get(i) == Some(&b'.') {
                i += 1;
                while raw.get(i).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                    digits += 1
                }
            }
            if digits == 0 {
                return Ok(DocumentValue::Real(0.));
            }
            if matches!(raw.get(i), Some(b'e' | b'E')) {
                let mut e = i + 1;
                if matches!(raw.get(e), Some(b'+' | b'-')) {
                    e += 1
                }
                let start = e;
                while raw.get(e).is_some_and(u8::is_ascii_digit) {
                    e += 1
                }
                if e > start {
                    i = e
                }
            }
            let value = token[..i].parse::<f64>()?;
            ensure!(value.is_finite(), "nonfinite document real");
            Ok(DocumentValue::Real(value))
        } else {
            let negative = token.starts_with('-');
            let digits = if token.starts_with(['-', '+']) {
                &token[1..]
            } else {
                token.as_str()
            };
            let mut number = 0u64;
            for c in digits.bytes().take_while(u8::is_ascii_digit) {
                number = number.saturating_mul(10).saturating_add((c - b'0') as u64)
            }
            Ok(DocumentValue::Integer(if negative {
                if number >= 1u64 << 63 {
                    i64::MIN
                } else {
                    -(number as i64)
                }
            } else {
                number.min(i64::MAX as u64) as i64
            }))
        }
    }
}
fn number_char(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')
}
/// Pretty-print ordered values with one-space indentation. LF is opt-in; the
/// native default is CRLF. Nonfinite values and excessive shapes fail visibly.
pub fn write_json(value: &DocumentValue, lf: bool) -> Result<String> {
    let mut out = String::new();
    let mut nodes = 0;
    fn emit(
        out: &mut String,
        v: &DocumentValue,
        depth: usize,
        lf: bool,
        nodes: &mut usize,
    ) -> Result<()> {
        *nodes += 1;
        ensure!(
            *nodes <= MAX_DOCUMENT_NODES && depth <= MAX_DOCUMENT_DEPTH,
            "document shape budget"
        );
        let newline = |out: &mut String, d: usize| {
            out.push_str(if lf { "\n" } else { "\r\n" });
            for _ in 0..d {
                out.push(' ')
            }
        };
        let quoted = |out: &mut String, s: &str| -> Result<()> {
            ensure!(s.len() <= MAX_DOCUMENT_BYTES, "document text budget");
            out.push_str(&serde_json::to_string(s)?);
            Ok(())
        };
        match v {
            DocumentValue::Void => out.push_str("null"),
            DocumentValue::Integer(i) => out.push_str(&i.to_string()),
            DocumentValue::Real(f) => {
                ensure!(f.is_finite(), "nonfinite document real");
                out.push_str(&f.to_string())
            }
            DocumentValue::Text(s) => quoted(out, s)?,
            DocumentValue::Array(values) => {
                out.push('[');
                newline(out, depth + 1);
                for (i, v) in values.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                        newline(out, depth + 1)
                    }
                    emit(out, v, depth + 1, lf, nodes)?
                }
                newline(out, depth);
                out.push(']')
            }
            DocumentValue::Object(values) => {
                out.push('{');
                newline(out, depth + 1);
                for (i, (k, v)) in values.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                        newline(out, depth + 1)
                    }
                    quoted(out, k)?;
                    out.push(':');
                    emit(out, v, depth + 1, lf, nodes)?
                }
                newline(out, depth);
                out.push('}')
            }
        }
        ensure!(
            out.len() <= MAX_DOCUMENT_BYTES * 4,
            "document output budget"
        );
        Ok(())
    }
    emit(&mut out, value, 0, lf, &mut nodes)?;
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_dialect_keeps_numeric_void_unicode_order_and_pretty_print_semantics() {
        let v = parse_json(
            "#comment\n{'a'=>1e5;'b':[true,,null,'\\uD83D\\uDE00',], 'a':2} trailing",
            JsonDialect::KiriKiri,
        )
        .unwrap();
        assert_eq!(
            v,
            DocumentValue::Object(vec![
                ("a".into(), DocumentValue::Integer(2)),
                (
                    "b".into(),
                    DocumentValue::Array(vec![
                        DocumentValue::Integer(1),
                        DocumentValue::Void,
                        DocumentValue::Void,
                        DocumentValue::Text("😀".into())
                    ])
                )
            ])
        );
        let s = write_json(&v, false).unwrap();
        assert!(s.contains("\r\n \"a\":2"));
        assert_eq!(parse_json(&s, JsonDialect::KiriKiri).unwrap(), v);
        assert_eq!(
            parse_json("1e5", JsonDialect::KiriKiri).unwrap(),
            DocumentValue::Integer(1)
        );
        assert_eq!(
            parse_json("9223372036854775808", JsonDialect::KiriKiri).unwrap(),
            DocumentValue::Integer(i64::MAX)
        );
    }
    #[test]
    fn malformed_or_excessive_documents_fail_without_ambient_state() {
        for s in ["{bare:1}", "/*", "'unterminated", "[1 2]", "'\\uD800'"] {
            assert!(parse_json(s, JsonDialect::KiriKiri).is_err(), "{s}")
        }
        assert!(parse_json(
            &format!("{}0{}", "[".repeat(70), "]".repeat(70)),
            JsonDialect::KiriKiri
        )
        .is_err());
        assert!(write_json(&DocumentValue::Real(f64::INFINITY), true).is_err());
    }
}

#[cfg(test)]
mod toml_tests {
    use super::*;
    #[test]
    fn toml_tables_arrays_and_scalars() {
        let v = parse_toml("a = 1\nb = \"x\"\n[t]\nf = 1.5\nok = true\n[[l]]\nn = 2\n").unwrap();
        let DocumentValue::Object(top) = v else { panic!() };
        assert_eq!(top[0], ("a".into(), DocumentValue::Integer(1)));
        assert_eq!(top[1], ("b".into(), DocumentValue::Text("x".into())));
        assert_eq!(
            top[2].1,
            DocumentValue::Object(vec![("f".into(), DocumentValue::Real(1.5)), ("ok".into(), DocumentValue::Integer(1))])
        );
        assert_eq!(top[3].1, DocumentValue::Array(vec![DocumentValue::Object(vec![("n".into(), DocumentValue::Integer(2))])]));
        assert!(parse_toml("a = ").is_err());
    }
}

