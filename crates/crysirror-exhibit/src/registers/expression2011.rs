// SPDX-License-Identifier: MPL-2.0
//! Numeric action parameters, Midori calcExpression (RVA 0xfe330).
use super::{Bank, Registers, Value};
use crate::native_numbers::integer2011;
use anyhow::{Context, Result, bail, ensure};

#[derive(Debug, PartialEq)]
pub(crate) enum Expression {
    Scalar(Operand),
    Postfix(Vec<Token>),
    Function(Function, Vec<Operand>),
}

#[derive(Debug, PartialEq)]
pub(crate) enum Token {
    Value(Operand),
    Operator(u8),
}

#[derive(Debug, PartialEq)]
pub(crate) struct Operand {
    source: Value,
    // Each outer M prefix uses the inner value as an experience identifier.
    banks: Vec<Option<Bank>>,
    local_bit: Option<i32>,
    windows_version: bool,
}

impl Operand {
    pub(super) fn immediate(value: i32) -> Self {
        Self {
            source: Value::Immediate(value),
            banks: Vec::new(),
            local_bit: None,
            windows_version: false,
        }
    }

    fn parse(mut raw: &[u8]) -> Result<Self> {
        let mut banks = Vec::new();
        while matches!(raw.first(), Some(b'M' | b'm')) {
            let bank = match raw.get(1).map(u8::to_ascii_uppercase) {
                Some(b'G') => Some(Bank::Cg),
                Some(b'S') => Some(Bank::Scene),
                Some(b'M') => Some(Bank::Sound),
                Some(b'V') => None,
                _ => break,
            };
            banks.push(bank);
            raw = &raw[2..];
        }
        let head = raw.first().map(u8::to_ascii_uppercase);
        let suffix = raw.get(1..).unwrap_or_default();
        let number = || integer2011(suffix);
        let mut local_bit = None;
        let mut windows_version = false;
        let source = match head {
            Some(b'R') => Value::Local(number()),
            Some(b'S') => Value::RelativeSystem(number()),
            Some(b'L') => Value::RelativeSystem(number().wrapping_add(1_000_000)),
            Some(b'C') => Value::SystemFlag(number() & 31),
            Some(b'F') => Value::Flag(number() & 31),
            Some(b'B') if matches!(suffix.first(), Some(b'R' | b'r')) => {
                let mut fields = suffix[1..].splitn(2, |&b| b == b'.');
                let register = integer2011(fields.next().unwrap_or_default());
                local_bit = Some(integer2011(fields.next().unwrap_or_default()));
                Value::Local(register)
            }
            Some(b'Q') => match number() {
                3 => {
                    windows_version = true;
                    Value::Immediate(0)
                }
                query @ (0..=2 | 4..=5 | 10..=18 | 20..=29) => {
                    bail!("unsupported native ExHibit Q query {query}");
                }
                _ => Value::Immediate(0),
            },
            // Unknown leading bytes take FCString's ordinary numeric conversion.
            _ => Value::Immediate(integer2011(raw)),
        };
        Ok(Self {
            source,
            banks,
            local_bit,
            windows_version,
        })
    }

    fn read(&self, regs: &Registers) -> Result<i32> {
        // liteStr2Value uses the original UxOsVersion platform/major/minor
        // tuple. The portable host has no Win32 compatibility identity by default.
        let mut value = if self.windows_version {
            match regs.windows_version {
                Some([1, 4, 0]) => 1,
                Some([1, 4, 10]) => 2,
                Some([1, 4, 90]) => 3,
                Some([2, 4, 0]) => 100,
                Some([2, 5, minor @ 0..=2]) => 101 + i32::from(minor),
                Some([2, 6, minor @ 0..=1]) => 104 + i32::from(minor),
                _ => -1,
            }
        } else {
            regs.read(self.source)?
        };
        if let Some(bit) = self.local_bit {
            value = i32::from((0..32).contains(&bit) && value & (1 << bit) != 0);
        }
        for bank in self.banks.iter().rev() {
            value = match bank {
                Some(bank) => regs.read(Value::Seen(*bank, value))?,
                None => {
                    let index = value as usize;
                    let word = regs.messages.get(&(index / 32)).copied().unwrap_or(0);
                    i32::from(word & (1 << ((index % 32) ^ 7)) != 0)
                }
            };
        }
        Ok(value)
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum Function {
    Modulo,
    DistanceSquared,
    Direction8,
    WithinRadius,
    Clip,
    EmptyText,
    HasText,
    CursorFlag,
}

impl Function {
    fn parse(raw: &[u8]) -> Result<Expression> {
        let open = raw
            .iter()
            .position(|&b| b == b'(')
            .context("missing ExHibit function argument list")?;
        let name = &raw[1..open];
        // calcFunction compares two bytes for short names and the first
        // four bytes for long names. Function prefixes are case-sensitive.
        let prefix = &name[..name.len().min(4)];
        let (function, arity, literal) = match prefix {
            b"mo" | b"mod" => (Self::Modulo, 2, false),
            b"dist" => (Self::DistanceSquared, 4, false),
            b"dir8" => (Self::Direction8, 4, false),
            b"isNe" => (Self::WithinRadius, 5, false),
            b"clip" => (Self::Clip, 4, false),
            b"isSt" => (Self::EmptyText, 1, true),
            b"isNo" => (Self::HasText, 1, true),
            b"isCu" => (Self::CursorFlag, 1, true),
            _ if name.len() == 3 && name.starts_with(b"mo") => (Self::Modulo, 2, false),
            _ => bail!(
                "unsupported ExHibit expression function {}",
                String::from_utf8_lossy(name)
            ),
        };
        let mut fields = raw[open + 1..]
            .split(|&b| matches!(b, b'\'' | b' ' | b'\t' | b')'))
            .filter(|s| !s.is_empty());
        let declared = crate::native_numbers::integer2011(fields.next().unwrap_or_default());
        ensure!(
            declared.max(1) >= arity,
            "insufficient native expression arguments"
        );
        let mut args = Vec::with_capacity(arity as usize);
        for _ in 0..arity {
            let field = if declared > 0 {
                fields.next().unwrap_or_default()
            } else {
                &[]
            };
            args.push(if literal {
                Operand::immediate(integer2011(field))
            } else {
                Operand::parse(field)?
            });
        }
        Ok(Expression::Function(function, args))
    }

    fn evaluate(&self, regs: &Registers, args: &[Operand]) -> Result<i32> {
        let mut values = [0; 5];
        for (slot, value) in values.iter_mut().zip(args) {
            *slot = value.read(regs)?;
        }
        let [a, b, c, d, e] = values;
        let squared = |x: i32, y: i32| x.wrapping_mul(x).wrapping_add(y.wrapping_mul(y));
        Ok(match self {
            Self::Modulo => {
                if b > 0 {
                    a % b
                } else {
                    0
                }
            }
            Self::DistanceSquared => squared(c.wrapping_sub(a), d.wrapping_sub(b)),
            Self::WithinRadius => {
                i32::from(a.wrapping_mul(a) >= squared(d.wrapping_sub(b), e.wrapping_sub(c)))
            }
            Self::Direction8 => {
                use std::cmp::Ordering::{Equal, Greater, Less};
                match (a.cmp(&c), b.cmp(&d)) {
                    (Equal, Equal) => 0,
                    (Equal, Greater) => 1,
                    (Less, Greater) => 2,
                    (Less, Equal) => 3,
                    (Less, Less) => 4,
                    (Equal, Less) => 5,
                    (Greater, Less) => 6,
                    (Greater, Equal) => 7,
                    (Greater, Greater) => 8,
                }
            }
            Self::Clip if a < b => {
                if d & 1 == 0 {
                    b
                } else {
                    c
                }
            }
            Self::Clip if a > c => {
                if d & 1 == 0 {
                    c
                } else {
                    b
                }
            }
            Self::Clip => a,
            Self::EmptyText => i32::from(regs.text(a)?.is_empty()),
            Self::HasText => i32::from(!regs.text(a)?.is_empty()),
            Self::CursorFlag => regs.read(Value::SystemFlag(3))? ^ i32::from(a == 0),
        })
    }
}

impl From<Value> for Expression {
    fn from(source: Value) -> Self {
        Self::Scalar(Operand {
            source,
            banks: Vec::new(),
            local_bit: None,
            windows_version: false,
        })
    }
}

impl Expression {
    pub(crate) fn parse(raw: &[u8]) -> Result<Self> {
        let raw = raw.trim_ascii();
        if raw.first() == Some(&b'@') {
            return Function::parse(raw);
        }
        if raw.first() == Some(&b'-') || !raw.iter().any(|b| b"*+-/".contains(b)) {
            return Ok(Self::Scalar(Operand::parse(raw)?));
        }
        let mut tokens = Vec::new();
        for word in raw.split(|&b| b == b' ').filter(|word| !word.is_empty()) {
            ensure!(
                tokens.len() < 65_536,
                "native expression exceeds token budget"
            );
            tokens.push(if b"*+-/".contains(&word[0]) {
                Token::Operator(word[0])
            } else {
                Token::Value(Operand::parse(word)?)
            });
        }
        Ok(Self::Postfix(tokens))
    }

    pub(crate) fn eval(&self, regs: &Registers) -> Result<i32> {
        let tokens = match self {
            Self::Scalar(value) => return value.read(regs),
            Self::Function(function, args) => return function.evaluate(regs, args),
            Self::Postfix(tokens) => tokens,
        };
        let mut stack = Vec::new();
        // The native pop at RVA 0xda890 leaves the caller's output cell
        // unchanged on an empty stack. Left and right are initialized once.
        let (mut left, mut right) = (0i32, 0i32);
        for token in tokens {
            match token {
                Token::Value(v) => {
                    left = v.read(regs)?;
                    stack.push(left);
                }
                Token::Operator(op) => {
                    if let Some(value) = stack.pop() {
                        right = value;
                    }
                    if let Some(value) = stack.pop() {
                        left = value;
                    }
                    left = match op {
                        b'+' => left.wrapping_add(right),
                        b'-' => left.wrapping_sub(right),
                        b'*' => left.wrapping_mul(right),
                        b'/' if right == 0 => return Ok(0),
                        b'/' => left
                            .checked_div(right)
                            .context("ExHibit expression signed divide overflow")?,
                        _ => unreachable!(),
                    };
                    stack.push(left);
                }
            }
        }
        Ok(stack.pop().unwrap_or(0))
    }
}
