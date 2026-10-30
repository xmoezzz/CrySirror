// SPDX-License-Identifier: GPL-2.0-or-later
//! Native text syntax and execution data. No resources, pixels or device state.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_SCRIPT: usize = 16 * 1024 * 1024;
#[derive(Clone, Debug)]
pub struct Statement {
    pub line: usize,
    pub body: String,
}
#[derive(Clone, Debug)]
pub struct Program {
    pub statements: Vec<Statement>,
    pub labels: BTreeMap<String, usize>,
}
impl Program {
    pub fn parse(text: &str) -> Result<Self> {
        ensure!(
            text.len() <= MAX_SCRIPT && !text.contains('\0'),
            "native script byte/NUL limit"
        );
        let mut p = Self {
            statements: vec![],
            labels: BTreeMap::new(),
        };
        let lines: Vec<&str> = text.lines().collect();
        let mut index = 0;
        while index < lines.len() {
            let line = index + 1;
            let mut raw = lines[index]
                .trim_start_matches('\u{feff}')
                .trim_start()
                .trim_end_matches('\r')
                .to_owned();
            index += 1;
            if raw.is_empty() || raw.starts_with(';') {
                continue;
            }
            if raw.starts_with('*') {
                let name = raw[1..]
                    .split(|c: char| c.is_whitespace() || c == ';')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                ensure!(!name.is_empty() && name.len() <= 128, "native label name");
                ensure!(
                    name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                    "unsupported native label characters"
                );
                let tail = raw[1 + name.len()..].trim_start();
                ensure!(
                    tail.is_empty() || tail.starts_with(';'),
                    "native label must occupy its own line"
                );
                ensure!(
                    p.labels.insert(name, p.statements.len()).is_none(),
                    "duplicate native label"
                );
                continue;
            }
            if raw.starts_with('`')
                || !raw.as_bytes()[0].is_ascii()
                || raw.starts_with('!')
                || raw.starts_with('#')
                || raw == "\\"
                || raw == "@"
            {
                append_text(&mut p.statements, line, &raw)?;
            } else {
                // Native select arguments can continue over physical newlines.
                if ["select ", "selgosub ", "selnum "]
                    .iter()
                    .any(|pre| raw.starts_with(pre))
                {
                    while raw.trim_end().ends_with(',') && index < lines.len() {
                        let next = lines[index].trim();
                        index += 1;
                        if next.is_empty() || next.starts_with(';') {
                            continue;
                        }
                        raw.push(' ');
                        raw.push_str(next);
                    }
                }
                let mut quote = None;
                let mut start = 0;
                for (at, ch) in raw.char_indices() {
                    if ch == '"' || ch == '`' {
                        if quote == Some(ch) {
                            quote = None;
                        } else if quote.is_none() {
                            quote = Some(ch);
                        }
                    }
                    if quote.is_none() && (ch == ':' || ch == ';') {
                        let body = raw[start..at].trim();
                        if !body.is_empty() {
                            p.statements.push(Statement {
                                line,
                                body: body.into(),
                            });
                        }
                        start = at + 1;
                        if ch == ';' {
                            start = raw.len();
                            break;
                        }
                    }
                }
                // Argument lexing is deferred until native execution. Some
                // installed scripts contain malformed strings in unreachable
                // branches; they must fail at that PC, not prevent boot.

                if start < raw.len() {
                    let body = raw[start..].trim();
                    if !body.is_empty() {
                        p.statements.push(Statement {
                            line,
                            body: body.into(),
                        });
                    }
                }
            }
            ensure!(p.statements.len() <= 524288, "native statement budget");
        }
        ensure!(!p.statements.is_empty(), "empty native script");
        Ok(p)
    }
    pub fn label(&self, name: &str) -> Result<usize> {
        self.labels
            .get(&name.trim_start_matches('*').to_ascii_lowercase())
            .copied()
            .with_context(|| format!("native label `{name}` missing"))
    }
}
// Split native text controls without moving drawing or reveal algorithms into the VM.
// Internal statements retain the original physical line for skip/error semantics.
fn append_text(out: &mut Vec<Statement>, line: usize, raw: &str) -> Result<()> {
    let raw = raw.strip_prefix('`').unwrap_or(raw);
    let mut start = 0;
    let mut at = 0;
    let mut terminal = false;
    while at < raw.len() {
        let c = raw[at..].chars().next().unwrap();
        let control = matches!(c, '@' | '\\')
            || c == '!'
                && raw
                    .as_bytes()
                    .get(at + 1)
                    .is_some_and(|b| matches!(*b, b's' | b'w' | b'd'))
            || c == '#'
                && raw
                    .get(at + 1..at + 7)
                    .is_some_and(|v| v.bytes().all(|b| b.is_ascii_hexdigit()));
        if !control {
            at += c.len_utf8();
            continue;
        }
        if at > start {
            out.push(Statement {
                line,
                body: format!("\u{1f}{}", &raw[start..at]),
            });
        }
        terminal = false;
        let (end, body) = match c {
            '@' => (at + 1, "click".to_owned()),
            '\\' => {
                terminal = true;
                (at + 1, "newpage".to_owned())
            }
            '#' => (at + 7, format!("textcolor {}", &raw[at..at + 7])),
            '!' => {
                let kind = raw
                    .as_bytes()
                    .get(at + 1)
                    .copied()
                    .context("native text control missing")?;
                let mut end = at + 2;
                if kind == b's' && raw.as_bytes().get(end) == Some(&b'd') {
                    end += 1;
                    (end, "textspeed 20".into())
                } else {
                    while raw.as_bytes().get(end).is_some_and(u8::is_ascii_digit) {
                        end += 1;
                    }
                    ensure!(end > at + 2, "native text control argument");
                    let command = match kind {
                        b's' => "textspeed",
                        b'w' => "wait",
                        b'd' => "delay",
                        _ => bail!("unsupported text control"),
                    };
                    (end, format!("{command} {}", &raw[at + 2..end]))
                }
            }
            _ => unreachable!(),
        };
        out.push(Statement { line, body });
        at = end;
        start = end;
    }
    let tail = &raw[start..];
    if !tail.is_empty() {
        let tail = tail.strip_suffix('/').unwrap_or(tail);
        if !tail.is_empty() {
            out.push(Statement {
                line,
                body: format!("\u{1f}{tail}"),
            });
        }
        terminal = raw.ends_with('/');
    } else if raw.ends_with('@') {
        terminal = true;
    }
    if start == raw.len() && (raw.starts_with('!') || raw.starts_with('#')) {
        terminal = true;
    }
    if !terminal {
        out.push(Statement {
            line,
            body: "br".into(),
        });
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Int(i32),
    Str(String),
}
impl Value {
    pub fn int(&self) -> Result<i32> {
        if let Self::Int(n) = self {
            Ok(*n)
        } else {
            bail!("native integer required")
        }
    }
    pub fn string(&self) -> Result<String> {
        if let Self::Str(s) = self {
            Ok(s.clone())
        } else {
            bail!("native string required")
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Num(i32),
    Str(String),
    IntVar(u16),
    StrVar(u16),
    IntAlias(String),
    StrAlias(String),
    Word(String),
    Label(String),
    Op(String),
    Comma,
}
#[derive(Clone, Debug)]
pub struct Tokens {
    pub values: Vec<Token>,
    pub at: usize,
}
impl Tokens {
    pub fn new(text: &str) -> Result<Self> {
        let mut values = Vec::new();
        let mut s = text;
        while !s.is_empty() {
            s = s.trim_start();
            if s.is_empty() {
                break;
            }
            let c = s.chars().next().unwrap();
            if c == '"' || c == '`' {
                let end = s[1..].find(c).context("unterminated quoted value")? + 1;
                values.push(Token::Str(s[1..end].into()));
                s = &s[end + 1..];
            } else if c == '%' || c == '$' {
                let end = 1 + s[1..]
                    .bytes()
                    .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                    .count();
                ensure!(end > 1, "native variable index/alias required");
                let name = &s[1..end];
                let token = if name.bytes().all(|b| b.is_ascii_digit()) {
                    let n: u16 = name.parse()?;
                    ensure!(n < 10000, "native variable index");
                    if c == '%' {
                        Token::IntVar(n)
                    } else {
                        Token::StrVar(n)
                    }
                } else if c == '%' {
                    Token::IntAlias(name.to_ascii_lowercase())
                } else {
                    Token::StrAlias(name.to_ascii_lowercase())
                };
                values.push(token);
                s = &s[end..];
            } else if c == '#' {
                ensure!(
                    s.len() >= 7 && s[1..7].bytes().all(|b| b.is_ascii_hexdigit()),
                    "native RGB colour"
                );
                values.push(Token::Str(s[..7].to_ascii_lowercase()));
                s = &s[7..];
            } else if c.is_ascii_digit() {
                let end = s.bytes().take_while(u8::is_ascii_digit).count();
                values.push(Token::Num(s[..end].parse()?));
                s = &s[end..];
            } else if c.is_ascii_alphabetic()
                || c == '_'
                || c == '*'
                    && values.last().is_none_or(|previous| match previous {
                        Token::Comma => true,
                        Token::Word(w) => matches!(
                            w.as_str(),
                            "goto" | "gosub" | "return" | "trap" | "lr_trap" | "r_trap"
                        ),
                        Token::Op(op) => op != ")",
                        _ => false,
                    })
                    && s.as_bytes()
                        .get(1)
                        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
            {
                let label = c == '*';
                let begin = usize::from(label);
                let end = begin
                    + s[begin..]
                        .bytes()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_')
                        .count();
                let name = s[begin..end].to_ascii_lowercase();
                values.push(if label {
                    Token::Label(name)
                } else {
                    Token::Word(name)
                });
                s = &s[end..];
            } else if c == ',' {
                values.push(Token::Comma);
                s = &s[1..];
            } else if "+-*/()=<>!&|".contains(c) {
                let two = s.get(..2).unwrap_or("");
                let n = if matches!(two, "<=" | ">=" | "==" | "!=" | "<>" | "&&" | "||") {
                    2
                } else {
                    1
                };
                values.push(Token::Op(s[..n].into()));
                s = &s[n..];
            } else {
                bail!("unsupported native token `{c}`");
            }
            ensure!(values.len() <= 2048, "native token budget");
        }
        Ok(Self { values, at: 0 })
    }
    pub fn peek(&self) -> Option<&Token> {
        self.values.get(self.at)
    }
    pub fn take(&mut self) -> Result<Token> {
        let t = self.peek().cloned().context("missing native argument")?;
        self.at += 1;
        Ok(t)
    }
    pub fn comma(&mut self) -> Result<()> {
        ensure!(self.take()? == Token::Comma, "native comma required");
        Ok(())
    }
    pub fn end(&self) -> Result<()> {
        ensure!(
            self.at == self.values.len(),
            "unsupported trailing native arguments"
        );
        Ok(())
    }
    pub fn word(&mut self) -> Result<String> {
        if let Token::Word(s) = self.take()? {
            Ok(s)
        } else {
            bail!("native command/alias name required")
        }
    }
    pub fn variable(&mut self) -> Result<Token> {
        let t = self.take()?;
        ensure!(
            matches!(
                t,
                Token::IntVar(_) | Token::StrVar(_) | Token::IntAlias(_) | Token::StrAlias(_)
            ),
            "native destination variable required"
        );
        Ok(t)
    }
    pub fn expr(&mut self, vars: &Variables) -> Result<Value> {
        self.binary(vars, 0)
    }
    fn binary(&mut self, vars: &Variables, min: u8) -> Result<Value> {
        let mut left = match self.take()? {
            Token::Num(n) => Value::Int(n),
            Token::Str(s) | Token::Label(s) => Value::Str(s),
            Token::IntAlias(ref name) => {
                let n = vars.index(name)?;
                Value::Int(*vars.ints.get(&n).unwrap_or(&0))
            }
            Token::StrAlias(ref name) => {
                let n = vars.index(name)?;
                Value::Str(vars.strings.get(&n).cloned().unwrap_or_default())
            }
            Token::IntVar(n) => Value::Int(*vars.ints.get(&n).unwrap_or(&0)),
            Token::StrVar(n) => Value::Str(vars.strings.get(&n).cloned().unwrap_or_default()),
            Token::Word(w) => vars
                .aliases
                .get(&w)
                .cloned()
                .with_context(|| format!("native alias `{w}` missing"))?,
            Token::Op(op) if op == "-" => Value::Int(self.binary(vars, 3)?.int()?.wrapping_neg()),
            Token::Op(op) if op == "+" => Value::Int(self.binary(vars, 3)?.int()?),
            Token::Op(op) if op == "(" => {
                let value = self.expr(vars)?;
                ensure!(
                    self.take()? == Token::Op(")".into()),
                    "native closing parenthesis"
                );
                value
            }
            token => bail!("invalid native expression {token:?}"),
        };
        loop {
            let Some(Token::Op(op)) = self.peek() else {
                break;
            };
            let prec = match op.as_str() {
                "+" | "-" => 1,
                "*" | "/" => 2,
                _ => break,
            };
            if prec < min {
                break;
            }
            let op = op.clone();
            self.at += 1;
            let right = self.binary(vars, prec + 1)?;
            left = match (left, right, op.as_str()) {
                (Value::Str(a), Value::Str(b), "+") => {
                    ensure!(a.len() + b.len() <= 65536, "native string budget");
                    Value::Str(a + &b)
                }
                (Value::Int(a), Value::Int(b), op) => Value::Int(match op {
                    "+" => a.wrapping_add(b),
                    "-" => a.wrapping_sub(b),
                    "*" => a.wrapping_mul(b),
                    "/" => {
                        ensure!(b != 0, "native division by zero");
                        a.wrapping_div(b)
                    }
                    _ => unreachable!(),
                }),
                _ => bail!("mixed native string/integer arithmetic"),
            };
        }
        Ok(left)
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Variables {
    pub ints: BTreeMap<u16, i32>,
    pub strings: BTreeMap<u16, String>,
    pub aliases: BTreeMap<String, Value>,
}
impl Variables {
    pub fn index(&self, name: &str) -> Result<u16> {
        let n = self
            .aliases
            .get(name)
            .with_context(|| format!("native variable alias `{name}` missing"))?
            .int()?;
        ensure!((0..10000).contains(&n), "native variable alias index");
        Ok(n as u16)
    }
    pub fn resolve(&self, dest: Token) -> Result<Token> {
        Ok(match dest {
            Token::IntAlias(name) => Token::IntVar(self.index(&name)?),
            Token::StrAlias(name) => Token::StrVar(self.index(&name)?),
            t => t,
        })
    }
    pub fn set(&mut self, dest: Token, value: Value) -> Result<()> {
        match self.resolve(dest)? {
            Token::IntVar(n) => {
                self.ints.insert(n, value.int()?);
            }
            Token::StrVar(n) => {
                let s = value.string()?;
                ensure!(s.len() <= 65536, "native string budget");
                self.strings.insert(n, s);
            }
            _ => bail!("invalid native destination"),
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expressions_keep_precedence_variables_aliases_and_comparisons_separate() {
        let mut vars = Variables::default();
        vars.ints.insert(0, 7);
        vars.aliases.insert("scale".into(), Value::Int(3));
        let mut tokens = Tokens::new("%0+2*scale >= 12 goto *ok").unwrap();
        assert_eq!(tokens.expr(&vars).unwrap(), Value::Int(13));
        assert_eq!(tokens.take().unwrap(), Token::Op(">=".into()));
        assert_eq!(tokens.expr(&vars).unwrap(), Value::Int(12));
        assert_eq!(tokens.word().unwrap(), "goto");
        assert_eq!(tokens.expr(&vars).unwrap(), Value::Str("ok".into()));
        assert_eq!(
            Tokens::new("\"a\"+\"b\"").unwrap().expr(&vars).unwrap(),
            Value::Str("ab".into())
        );
        assert!(Tokens::new("10/0").unwrap().expr(&vars).is_err());
    }
    #[test]
    fn native_statement_delimiters_respect_quotes_and_unknown_data_is_not_silently_skipped() {
        let p = Program::parse("*start\nmov $0,\":a;test.png\": click ; comment\nend\n").unwrap();
        assert_eq!(p.statements.len(), 3);
        assert_eq!(p.statements[0].body, "mov $0,\":a;test.png\"");
        assert!(Program::parse("*start\n*start\nend").is_err());
        assert!(Program::parse("*start: mov %0,1\nend").is_err());
        assert!(Program::parse("*start unknown command\nend").is_err());
    }
}

#[cfg(test)]
mod extended_tests {
    use super::*;
    #[test]
    fn aliases_backticks_and_native_inline_waits_keep_physical_line_addresses() {
        let p = Program::parse(
            "*start\n`First!@ next\\\nselect `Yes`,*yes,\n `No`,*no\n*yes\nend\n*no\nend\n",
        )
        .unwrap();
        assert_eq!(p.statements[0].body, "\u{1f}First!");
        assert_eq!(p.statements[1].body, "click");
        assert_eq!(p.statements[3].body, "newpage");
        assert_eq!(p.statements[3].line, 2);
        assert!(p.statements[4].body.contains("`No`"));
        let mut vars = Variables::default();
        vars.aliases.insert("flag".into(), Value::Int(350));
        vars.set(Token::IntAlias("flag".into()), Value::Int(7))
            .unwrap();
        let mut tokens = Tokens::new("%flag+1").unwrap();
        assert_eq!(tokens.expr(&vars).unwrap(), Value::Int(8));
        assert_eq!(
            Tokens::new("`a,b`+#ff0000").unwrap().expr(&vars).unwrap(),
            Value::Str("a,b#ff0000".into())
        );
    }
}
