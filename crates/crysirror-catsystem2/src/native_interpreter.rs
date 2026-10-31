// SPDX-License-Identifier: MPL-2.0
//! kcInterpriter's original variable and expression state, not a hosted engine.
//!
//! Reviewed owner module: 728460/728970/728990/7289d0, 728ad0 through
//! 729c20, and the lexer at 877640 through 878760. Logical node indices replace
//! the original pointers. No resource, rendering or audio implementation lives
//! here. The uninitialised native backup allocation is deliberately not exposed
//! as a fabricated zero-filled save bank.
use anyhow::{bail, ensure, Context, Result};
use std::collections::BTreeMap;

const MAX_FLAGS: usize = 1_048_576;
const COMMAND_SLOTS: usize = 22528 / 88;

#[derive(Clone, Debug)]
pub(crate) struct Interpreter {
    flags: Vec<u32>,
    backup: Vec<u32>,
    backup_initialized: bool,
    default_value: u32,
    dirty: bool,
    locals: BTreeMap<u32, LocalBank>,
    selected: Option<u32>,
    parser: Parser,
    commands: Vec<Token>,
}
#[derive(Clone, Debug)]
struct LocalBank {
    variables: Vec<u32>,
    names: BTreeMap<(Vec<u8>, u32), u32>,
}
impl Interpreter {
    pub(crate) fn new(count: u32) -> Result<Self> {
        ensure!(
            (count as usize) <= MAX_FLAGS,
            "CatSystem2 flag allocation budget"
        );
        Ok(Self {
            flags: vec![0; count as usize],
            backup: vec![0; count as usize],
            backup_initialized: false,
            default_value: 0,
            dirty: false,
            locals: BTreeMap::new(),
            selected: None,
            parser: Parser::default(),
            commands: Vec::with_capacity(COMMAND_SLOTS),
        })
    }
    pub(crate) fn flag(&self, index: u32) -> u32 {
        self.flags.get(index as usize).copied().unwrap_or(0)
    }
    pub(crate) fn set_flag(&mut self, index: u32, value: u32) -> bool {
        // The native dirty bit is set even for a rejected index.
        self.dirty = true;
        if let Some(slot) = self.flags.get_mut(index as usize) {
            *slot = value;
            true
        } else {
            false
        }
    }
    pub(crate) fn set_default(&mut self, value: u32) {
        self.default_value = value;
    }
    pub(crate) fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"flags":self.flags.len(),"dirty":self.dirty,
            "backup_initialised":self.backup_initialized,"backup_flags":self.backup.len(),"default":self.default_value,
            "local_banks":self.locals.len(),"selected_local":self.selected,
            "command_capacity":COMMAND_SLOTS,"command_count":self.commands.len(),
            "parser_source_bytes":self.parser.source.len(),"parser_offset":self.parser.offset,
            "parser_line":self.parser.line,"parser_keyword_mode":false})
    }
    pub(crate) fn evaluate(&mut self, source: &[u8]) -> Result<u32> {
        // Parse and validate the complete tree before any flag side effects.
        let mut parser = self.parser.clone();
        if !parser.load(source)? {
            return Ok(0);
        }
        let mut tokens = Vec::new();
        while let Some(mut token) = parser.next()? {
            if token.kind == 6 || token.kind == 5 {
                continue;
            } // native whitespace/newline
            if token.kind == 39 {
                token = Token::number(self.default_value);
            }
            ensure!(
                tokens.len() < COMMAND_SLOTS,
                "CatSystem2 expression command memory exhausted"
            );
            ensure!(
                token.text.len() < 64,
                "CatSystem2 native command token text extent"
            );
            tokens.push(token);
        }
        let tree = Tree::compile(&tokens)?;
        let mut working = self.clone();
        let value = if let Some(tree) = tree {
            let value = working.eval(&tree, 0)?.value;
            // 729c20 unary+ stores the operand record but returns its initial0.
            if tree.token.kind == 176 {
                0
            } else {
                value
            }
        } else {
            0
        };
        working.parser = parser;
        working.commands = tokens;
        *self = working;
        Ok(value)
    }
    fn resolved(&self, value: Value) -> Result<u32> {
        match value.address {
            None => Ok(value.value),
            Some((false, index)) => self
                .flags
                .get(index as usize)
                .copied()
                .context("CatSystem2 expression global variable out of range"),
            Some((true, index)) => self
                .locals
                .get(
                    &self
                        .selected
                        .context("CatSystem2 expression has no selected local bank")?,
                )
                .and_then(|b| b.variables.get(index as usize))
                .copied()
                .context("CatSystem2 expression private variable out of range"),
        }
    }
    fn assign(&mut self, target: Value, value: u32) -> Result<()> {
        if let Some((private, index)) = target.address {
            if private {
                *self
                    .locals
                    .get_mut(
                        &self
                            .selected
                            .context("CatSystem2 expression has no selected local bank")?,
                    )
                    .and_then(|b| b.variables.get_mut(index as usize))
                    .context("CatSystem2 expression private variable out of range")? = value;
            } else {
                ensure!(
                    (index as usize) < self.flags.len(),
                    "CatSystem2 expression global variable out of range"
                );
                self.set_flag(index, value);
            }
        }
        Ok(())
    }
    fn eval(&mut self, tree: &Tree, depth: usize) -> Result<Value> {
        ensure!(depth <= COMMAND_SLOTS, "CatSystem2 expression tree depth");
        let token = &tree.token;
        if token.kind == 13 {
            return Ok(Value::immediate(token.value));
        }
        if token.kind == 7 {
            let index = tree
                .right
                .as_ref()
                .map(|t| self.eval(t, depth + 1).and_then(|v| self.resolved(v)))
                .transpose()?
                .unwrap_or(0);
            let value = self
                .selected
                .and_then(|id| self.locals.get(&id))
                .and_then(|b| b.names.get(&(token.text.clone(), index)))
                .copied()
                .unwrap_or(0);
            return Ok(Value::immediate(value));
        }
        let left = tree
            .left
            .as_ref()
            .map(|t| self.eval(t, depth + 1))
            .transpose()?
            .unwrap_or(Value::immediate(0));
        let right = tree
            .right
            .as_ref()
            .map(|t| self.eval(t, depth + 1))
            .transpose()?
            .unwrap_or(Value::immediate(0));
        if (1..=4).contains(&token.kind) {
            let index = self.resolved(right)?;
            let value = Value {
                value: 0,
                address: Some((token.kind >= 3, index)),
            };
            return Ok(Value {
                value: self.resolved(value)?,
                ..value
            });
        }
        // Both operands have already executed: the original && and || do not
        // short-circuit. Native value records retain lvalue references.
        let a = self.resolved(left)?;
        let b = self.resolved(right)?;
        let value = match token.kind {
            128 => (b == 0) as u32,
            129 => (a != b) as u32,
            130 => signed_rem(a, b)?,
            131 => a & b,
            132 => (a != 0 && b != 0) as u32,
            135 => a.wrapping_mul(b),
            136 => a.wrapping_add(b),
            137 | 141 => {
                let n = if token.kind == 137 {
                    b.wrapping_add(1)
                } else {
                    b.wrapping_sub(1)
                };
                self.assign(right, n)?;
                n
            }
            138 | 142 => {
                let n = if token.kind == 138 {
                    a.wrapping_add(1)
                } else {
                    a.wrapping_sub(1)
                };
                self.assign(left, n)?;
                a
            }
            140 => a.wrapping_sub(b),
            146 => {
                if b == 0 {
                    0
                } else {
                    signed_div(a, b)?
                }
            }
            149 => ((a as i32) < (b as i32)) as u32,
            150 => a.wrapping_shl(b),
            151 => ((a as i32) <= (b as i32)) as u32,
            152 => {
                self.assign(left, b)?;
                if left.address.is_some() {
                    b
                } else {
                    0
                }
            }
            153..=159 | 161..=163 => {
                let n = match token.kind {
                    153 => {
                        ensure!(b != 0, "CatSystem2 compound modulo by zero");
                        signed_rem(a, b)?
                    }
                    154 => a & b,
                    155 => a.wrapping_mul(b),
                    156 => a.wrapping_add(b),
                    157 => a.wrapping_sub(b),
                    158 => signed_div(a, b)?,
                    159 => a.wrapping_shl(b),
                    161 => (a as i32).wrapping_shr(b) as u32,
                    162 => a ^ b,
                    163 => a | b,
                    _ => unreachable!(),
                };
                self.assign(left, n)?;
                if left.address.is_some() {
                    n
                } else {
                    0
                }
            }
            160 => (a == b) as u32,
            164 => ((a as i32) > (b as i32)) as u32,
            165 => ((a as i32) >= (b as i32)) as u32,
            166 => (a as i32).wrapping_shr(b) as u32,
            170 => a ^ b,
            172 => a | b,
            173 => (a != 0 || b != 0) as u32,
            175 => !b,
            176 => return Ok(Value::immediate(b)),
            177 => b.wrapping_neg(),
            _ => bail!("CatSystem2 unsupported expression token {}", token.kind),
        };
        Ok(Value::immediate(value))
    }
}
fn signed_div(a: u32, b: u32) -> Result<u32> {
    ensure!(b != 0, "CatSystem2 expression division by zero");
    ensure!(
        a != 0x80000000 || b != u32::MAX,
        "CatSystem2 expression signed division overflow"
    );
    Ok(((a as i32) / (b as i32)) as u32)
}
fn signed_rem(a: u32, b: u32) -> Result<u32> {
    if b == 0 {
        return Ok(0);
    }
    ensure!(
        a != 0x80000000 || b != u32::MAX,
        "CatSystem2 expression signed remainder overflow"
    );
    Ok(((a as i32) % (b as i32)) as u32)
}
#[derive(Copy, Clone)]
struct Value {
    value: u32,
    address: Option<(bool, u32)>,
}
impl Value {
    fn immediate(value: u32) -> Self {
        Self {
            value,
            address: None,
        }
    }
}

#[derive(Clone, Debug)]
struct Token {
    kind: u32,
    value: u32,
    text: Vec<u8>,
}
impl Token {
    fn number(value: u32) -> Self {
        Self {
            kind: 13,
            value,
            text: Vec::new(),
        }
    }
}
#[derive(Clone, Debug)]
struct Tree {
    token: Token,
    left: Option<Box<Tree>>,
    right: Option<Box<Tree>>,
}
impl Tree {
    fn compile(tokens: &[Token]) -> Result<Option<Self>> {
        if tokens.is_empty() {
            return Ok(None);
        }
        let mut values: Vec<Tree> = Vec::new();
        let mut ops: Vec<Token> = Vec::new();
        for (index, original) in tokens.iter().enumerate() {
            let mut token = original.clone();
            let previous = index.checked_sub(1).map(|i| &tokens[i]);
            if matches!(token.kind, 136 | 140)
                && previous.is_none_or(|t| operator(t.kind) && !matches!(t.kind, 134 | 169))
            {
                token.kind = if token.kind == 136 { 176 } else { 177 };
            }
            if matches!(token.kind, 137 | 141)
                && previous.is_some_and(|t| t.kind == 7 || matches!(t.kind, 134 | 169))
            {
                token.kind += 1;
            }
            if matches!(token.kind, 7 | 13) {
                values.push(Self {
                    token,
                    left: None,
                    right: None,
                });
                continue;
            }
            ensure!(
                operator(token.kind),
                "CatSystem2 unsupported expression literal {}",
                token.kind
            );
            while let Some(top) = ops.last() {
                if precedence(top.kind).0 < precedence(token.kind).1 {
                    break;
                }
                if matches!((top.kind, token.kind), (133, 134) | (168, 169)) {
                    ops.pop();
                    break;
                }
                Self::reduce(&mut values, ops.pop().unwrap())?;
            }
            if !matches!(token.kind, 134 | 169) {
                ops.push(token);
            }
        }
        while let Some(op) = ops.pop() {
            Self::reduce(&mut values, op)?;
        }
        ensure!(values.len() == 1, "CatSystem2 malformed expression tree");
        Ok(values.pop())
    }
    fn reduce(values: &mut Vec<Tree>, token: Token) -> Result<()> {
        let (left, right) = match token.kind {
            1..=4 | 128 | 137 | 141 | 175..=177 => (
                None,
                Some(Box::new(
                    values
                        .pop()
                        .context("CatSystem2 missing expression operand")?,
                )),
            ),
            138 | 142 => (
                Some(Box::new(
                    values.pop().context("CatSystem2 missing postfix operand")?,
                )),
                None,
            ),
            133 | 168 => bail!("CatSystem2 unmatched expression bracket"),
            _ => {
                let right = values.pop().context("CatSystem2 missing expression RHS")?;
                let left = values.pop().context("CatSystem2 missing expression LHS")?;
                (Some(Box::new(left)), Some(Box::new(right)))
            }
        };
        values.push(Self { token, left, right });
        Ok(())
    }
}
fn operator(k: u32) -> bool {
    (1..=4).contains(&k) || (128..=177).contains(&k)
}
fn precedence(k: u32) -> (u32, u32) {
    match k {
        1..=4 => (30, 31),
        128 => (28, 29),
        175 => (0, 29),
        129 | 160 => (19, 18),
        130 | 135 | 146 => (27, 26),
        131 => (17, 16),
        132 => (11, 10),
        133 | 168 => (0, 33),
        134 | 169 => (32, 0),
        136 | 140 => (25, 24),
        137 | 141 => (29, 28),
        138 | 142 | 176 | 177 => (28, 29),
        148 => (1, 0),
        149 | 151 | 164 | 165 => (21, 20),
        150 | 166 => (23, 22),
        152..=159 | 161..=163 => (4, 5),
        170 => (15, 14),
        172 => (13, 12),
        173 => (9, 8),
        _ => (0, 0),
    }
}

#[derive(Clone, Debug)]
struct Parser {
    source: Vec<u8>,
    offset: usize,
    line: u32,
}
impl Default for Parser {
    fn default() -> Self {
        Self {
            source: Vec::new(),
            offset: 0,
            line: 0,
        }
    }
}
impl Parser {
    fn load(&mut self, source: &[u8]) -> Result<bool> {
        if source.is_empty() {
            return Ok(false);
        }
        ensure!(
            source.len() <= 65534 && !source.contains(&0),
            "CatSystem2 expression source budget/NUL"
        );
        self.source = source.to_vec();
        self.offset = 0;
        self.line = 1;
        Ok(true)
    }
    fn next(&mut self) -> Result<Option<Token>> {
        // Original lexer skips comments and backslash-CRLF before every token.
        loop {
            let s = &self.source[self.offset..];
            if s.is_empty() || matches!(s[0], 0 | 26) {
                return Ok(None);
            }
            if s.starts_with(b"\\\r\n") {
                self.offset += 3;
                continue;
            }
            if s.starts_with(b"//") {
                if let Some(end) = s.windows(2).position(|w| w == b"\r\n") {
                    self.offset += end;
                    continue;
                }
                return Ok(None);
            }
            if s.starts_with(b"/*") {
                if let Some(end) = s.windows(2).position(|w| w == b"*/") {
                    self.line += s[..end].windows(2).filter(|w| *w == b"\r\n").count() as u32;
                    self.offset += end + 2;
                    continue;
                }
                return Ok(None);
            }
            break;
        }
        let s = &self.source[self.offset..];
        let mut used = 1;
        let mut token = Token {
            kind: 0,
            value: 0,
            text: Vec::new(),
        };
        if matches!(s[0], b' ' | b'\t') {
            token.kind = 6;
        } else if matches!(s[0], b'\r' | b'\n') {
            token.kind = 5;
            used = if s.starts_with(b"\r\n") { 2 } else { 1 };
            self.line += 1;
        } else if s[0].is_ascii_digit() {
            let hex = s.starts_with(b"0x") || s.starts_with(b"0X");
            used = if hex { 2 } else { 0 };
            let mut value = 0u32;
            while let Some(digit) = s
                .get(used)
                .and_then(|b| (*b as char).to_digit(if hex { 16 } else { 10 }))
            {
                value = value
                    .wrapping_mul(if hex { 16 } else { 10 })
                    .wrapping_add(digit);
                used += 1;
            }
            ensure!(
                hex || s.get(used) != Some(&b'.'),
                "CatSystem2 float expression is unsupported"
            );
            token = Token::number(value);
        } else if s[0] == b'\'' {
            used = 1;
            let mut value = 0u16;
            let mut closed = false;
            while let Some(&b) = s.get(used) {
                if b == b'\'' {
                    used += 1;
                    closed = true;
                    break;
                }
                value = value.wrapping_shl(8);
                if b == b'\\' && s.get(used + 1).is_some_and(|b| matches!(b, b'\'' | b'\\')) {
                    used += 1;
                }
                value |= s[used] as i8 as i16 as u16;
                used += 1;
            }
            ensure!(closed, "CatSystem2 unterminated character constant");
            token = Token::number(value as u32);
        } else if s[0] == b'@' {
            token.kind = 39;
        } else if s[0].is_ascii_alphabetic() || s[0] == b'_' {
            used = s
                .iter()
                .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
                .count();
            token.kind = 7;
            token.text = s[..used].to_vec();
        } else {
            const OPS: &[(&[u8], u32)] = &[
                (b"<<=", 159),
                (b">>=", 161),
                (b"!=", 129),
                (b"%=", 153),
                (b"&&", 132),
                (b"&=", 154),
                (b"*=", 155),
                (b"++", 137),
                (b"+=", 156),
                (b"--", 141),
                (b"-=", 157),
                (b"/=", 158),
                (b"<<", 150),
                (b"<=", 151),
                (b"==", 160),
                (b">=", 165),
                (b">>", 166),
                (b"^=", 162),
                (b"|=", 163),
                (b"||", 173),
                (b"$$", 3),
                (b"!", 128),
                (b"#", 1),
                (b"$", 2),
                (b"\\", 4),
                (b"%", 130),
                (b"&", 131),
                (b"(", 133),
                (b")", 134),
                (b"*", 135),
                (b"+", 136),
                (b"-", 140),
                (b"/", 146),
                (b";", 148),
                (b"<", 149),
                (b"=", 152),
                (b">", 164),
                (b"[", 168),
                (b"]", 169),
                (b"^", 170),
                (b"|", 172),
                (b"~", 175),
            ];
            if let Some((bytes, kind)) = OPS.iter().find(|(bytes, _)| s.starts_with(bytes)) {
                used = bytes.len();
                token.kind = *kind;
            } else {
                bail!(
                    "CatSystem2 unsupported expression byte {:#x} at {}",
                    s[0],
                    self.offset
                );
            }
        }
        self.offset += used;
        Ok(Some(token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn constructor_owns_actual_banks_and_native_dirty_index_policy() {
        let mut i = Interpreter::new(4).unwrap();
        assert_eq!(i.flag(1), 0);
        assert!(!i.dirty);
        assert!(!i.backup_initialized);
        assert_eq!(i.backup.len(), 4);
        assert!(i.set_flag(1, 0xfffffff0));
        assert_eq!(i.flag(1), 0xfffffff0);
        assert!(i.dirty);
        i.dirty = false;
        assert!(!i.set_flag(4, 7));
        assert!(i.dirty);
        assert_eq!(i.flag(4), 0);
        assert!(Interpreter::new(MAX_FLAGS as u32 + 1).is_err());
    }
    #[test]
    fn original_expression_state_is_executable_and_transactional() {
        let mut i = Interpreter::new(16).unwrap();
        i.set_default(9);
        assert_eq!(i.evaluate(b"#2 = 3 + 4 * 5").unwrap(), 23);
        assert_eq!(i.flag(2), 23);
        assert_eq!(i.evaluate(b"#2 += @").unwrap(), 32);
        assert_eq!(i.flag(2), 32);
        assert_eq!(i.evaluate(b"(#2 << 1) | 3").unwrap(), 67);
        assert_eq!(i.evaluate(b"0 && ++#2").unwrap(), 0);
        assert_eq!(i.flag(2), 33);
        assert_eq!(i.evaluate(b"#[2]++").unwrap(), 33);
        assert_eq!(i.flag(2), 34);
        assert!(i.evaluate(b"(#2=99) + #9999").is_err());
        assert_eq!(i.flag(2), 34);
        assert_eq!(i.evaluate(b"+5").unwrap(), 0);
        assert_eq!(i.evaluate(b"(+5)+1").unwrap(), 6);
        assert_eq!(i.evaluate(b"~1+2").unwrap(), !3u32);
        assert_eq!(i.evaluate(b"0 || ++#2").unwrap(), 1);
        assert_eq!(i.flag(2), 35);
        assert_eq!(i.evaluate(b"-9 / 2").unwrap(), (-4i32) as u32);
        assert!(i.evaluate(b"#2=99 + unknown ???").is_err());
        assert_eq!(i.flag(2), 35);
        assert_eq!(
            i.evaluate(b"/*a\r\nb*/ #3 = 0xffffffff + 2 // tail\r\n")
                .unwrap(),
            1
        );
        assert_eq!(i.flag(3), 1);
        assert_eq!(i.parser.line, 3);
        assert!(i.evaluate(&vec![b'1'; 65535]).is_err());
    }
}
