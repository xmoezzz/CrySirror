// SPDX-License-Identifier: MPL-2.0
//! v1 RetouchSystem::liteCheckCondition and blockId2Index.
use super::{cstring, word};
use crate::registers::{Expression, Registers, Value};
use anyhow::{bail, ensure, Context, Result};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
pub(super) struct Position {
    pub pc: usize,
    pub remaining: u32,
}

/// End of an RLD record independently of its execution semantics.
pub(super) fn envelope(bytes: &[u8], pc: usize) -> Result<(u32, Option<&[u8]>, usize)> {
    let header = word(bytes, pc)?;
    let mut at = pc + 4 + usize::from((header >> 16) as u8) * 4;
    ensure!(at <= bytes.len(), "truncated RLD record at {pc:#x}");
    let mut first = None;
    for _ in 0..(header >> 24) & 15 {
        let (field, end) = cstring(bytes, at)?;
        first.get_or_insert(field);
        at = end;
    }
    Ok((header, first, at))
}

pub(super) fn labels(bytes: &[u8], mut pos: Position, abi: u32) -> Result<BTreeMap<i32, Position>> {
    let mut labels = BTreeMap::new();
    let start = pos;
    let mut block = 0i32;
    while pos.remaining > 0 {
        let (header, first, at) = envelope(bytes, pos.pc)?;
        let n = (header >> 16) as u8;
        let flags = (header >> 24) as u8;
        let operands = pos.pc + 4;
        match header as u16 {
            5 => {
                let id = if flags & (if abi == 3 { 0xf0 } else { 0x70 }) != 0 {
                    let id = first
                        .context("missing block definition")?
                        .split(|b| *b == b',')
                        .next()
                        .unwrap_or_default();
                    number(id)?
                } else {
                    ensure!(n > 0, "missing numeric block ID");
                    word(bytes, operands)? as i32
                };
                block = if id == -1 { block.wrapping_add(1) } else { id };
                labels.entry(block).or_insert(pos);
            }
            47 if n > 0 => {
                labels.entry(word(bytes, operands)? as i32).or_insert(pos);
            }
            _ => {}
        }
        pos = Position {
            pc: at,
            remaining: pos.remaining - 1,
        };
    }
    ensure!(pos.pc == bytes.len(), "trailing bytes after RLD records");
    labels.entry(0).or_insert(start);
    Ok(labels)
}

pub(super) fn number(raw: &[u8]) -> Result<i32> {
    Ok(std::str::from_utf8(raw.trim_ascii())?.parse()?)
}

#[derive(Debug, PartialEq)]
enum Comparison {
    Constant(bool),
    Predicate(Expression),
    Program(Expression, u8, Expression),
    Numeric(Value, u8, Value),
    Text(i32, bool, Vec<u8>),
}

#[derive(Debug, PartialEq)]
pub(super) struct Condition(Vec<Vec<Comparison>>);

impl Condition {
    pub fn text(raw: &[u8]) -> Result<Self> {
        Self::for_abi(raw, 1)
    }
    pub fn for_abi(raw: &[u8], abi: u32) -> Result<Self> {
        if raw == b"*" || (abi == 3 && raw.is_empty()) {
            return Ok(Self(vec![vec![]]));
        }
        let mut groups = Vec::new();
        for group in raw
            .split(|b| *b == b';')
            .take(if abi == 3 { 24 } else { 16 })
        {
            let mut terms = Vec::new();
            for term in group.split(|b| *b == b'&') {
                let term = term.trim_ascii();
                if abi == 3 && matches!(term, b"true" | b"false") {
                    terms.push(Comparison::Constant(term == b"true"));
                    continue;
                }
                if abi == 3 && term.starts_with(b"@") {
                    terms.push(Comparison::Predicate(Expression::parse(term)?));
                    continue;
                }
                let at = term
                    .windows(2)
                    .position(|p| p[0] == 0x81 && (0x81..=0x86).contains(&p[1]))
                    .context("unsupported ExHibit text condition")?;
                // CP932 full-width =, !=, <, >, <=, >= mapped to the
                // comparison numbers used by the binary condition records.
                let op = [0, 1, 5, 2, 4, 3][usize::from(term[at + 1] - 0x81)];
                let left = term[..at].trim_ascii();
                let right = term[at + 2..].trim_ascii();
                if abi == 3 {
                    // The original takes a separate locale-aware text comparison
                    // path for uppercase T; numeric conversion would invent a zero.
                    ensure!(!left.starts_with(b"T"), "unsupported native text comparison");
                    terms.push(Comparison::Program(Expression::parse(left)?, op, Expression::parse(right)?));
                } else {
                    terms.push(Comparison::Numeric(Value::expression(left)?, op, Value::expression(right)?));
                }
            }
            groups.push(terms);
        }
        Ok(Self(groups))
    }

    pub fn binary(values: &[i32], strings: &[&[u8]]) -> Result<Self> {
        ensure!(
            values.len() == 16 && strings.len() == 5,
            "invalid RLD binary condition layout"
        );
        let mut terms = Vec::new();
        for (v, s) in values[1..].chunks_exact(3).zip(strings) {
            let (kind, index, op) = (v[0], v[1], v[2]);
            if kind < 0 {
                continue;
            }
            match kind {
                0 | 1 if (0..=5).contains(&op) => {
                    let left = if kind == 0 {
                        Value::Local(index)
                    } else {
                        Value::RelativeSystem(index)
                    };
                    terms.push(Comparison::Numeric(left, op as u8, Value::expression(s)?));
                }
                2 if matches!(op, 0 | 1) => {
                    terms.push(Comparison::Text(index, op == 0, s.to_vec()))
                }
                _ => bail!("unsupported binary condition {kind}/{op}"),
            }
        }
        Ok(Self(vec![terms]))
    }
    pub fn matches(&self, regs: &Registers) -> Result<bool> {
        for group in &self.0 {
            let mut pass = true;
            for term in group {
                pass = match term {
                    Comparison::Constant(value) => *value,
                    Comparison::Predicate(expr) => expr.eval(regs)? != 0,
                    Comparison::Program(left, op, right) => compare(left.eval(regs)?, *op, right.eval(regs)?),
                    Comparison::Numeric(left, op, right) => compare(regs.read(*left)?, *op, regs.read(*right)?),
                    Comparison::Text(index, expected, bytes) => {
                        (regs.text(*index)? == bytes.as_slice()) == *expected
                    }
                };
                if !pass {
                    break;
                }
            }
            if pass {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

fn compare(a: i32, op: u8, b: i32) -> bool {
    match op {
        0 => a == b,
        1 => a != b,
        2 => a > b,
        3 => a >= b,
        4 => a <= b,
        5 => a < b,
        _ => unreachable!(),
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct Block {
    pub id: i32,
    pub marker: bool,
    pub title: Vec<u8>,
    pub false_target: i32,
    pub condition: Condition,
    pub checkpoint_flag: Option<bool>,
    pub checkpoint_skip: Option<bool>,
}
impl Block {
    pub fn binary(id: i32, marker: i32, title: &[u8]) -> Result<Self> {
        Ok(Self {
            id,
            title: title.to_vec(),
            marker: marker != 0,
            false_target: -1,
            condition: Condition::text(b"*")?,
            checkpoint_flag: None,
            checkpoint_skip: None,
        })
    }
    pub fn csv(raw: &[u8], abi: u32, extended: bool) -> Result<Self> {
        let fields: Vec<_> = raw
            .splitn(if extended { 7 } else { 5 }, |b| *b == b',')
            .collect();
        ensure!(
            fields.len() == if extended { 7 } else { 5 },
            "invalid ExHibit block layout"
        );
        let flags = number(fields[1])?;
        ensure!(
            abi != 3 || flags & 0x30 == 0,
            "unsupported ExHibit block message/control entry flags {flags:#x}"
        );
        Ok(Self {
            id: number(fields[0])?,
            title: fields[fields.len() - 1].to_vec(),
            marker: flags & 1 != 0,
            false_target: number(fields[2])?,
            condition: Condition::for_abi(fields[3], abi)?,
            checkpoint_flag: (abi == 3 && flags & 1 == 0).then_some(flags & 0x40 != 0),
            checkpoint_skip: (abi == 3 && flags & 1 == 0).then_some(flags & 2 != 0),
        })
    }
}
