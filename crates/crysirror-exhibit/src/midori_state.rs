// SPDX-License-Identifier: MPL-2.0
//! Scoped Midori 2011 script-owned state and the literal subset of RNF.
use crate::{
    branch::number,
    native_numbers::integer2011,
    registers::{Expression, db_operand},
};
use anyhow::{Context, Result, bail, ensure};
use std::collections::BTreeMap;

#[derive(Debug, PartialEq)]
pub(super) struct DbDefinition {
    pub table: i32,
    pub rows: BTreeMap<i32, [i32; 8]>,
}

impl DbDefinition {
    pub fn parse(table: i32, rows: &[u8]) -> Result<Self> {
        if !(0..8).contains(&table) {
            return Ok(Self {
                table,
                rows: BTreeMap::new(),
            });
        }
        ensure!(rows.len() <= 65536, "oversized ExHibit integer table");
        let mut output = BTreeMap::new();
        for row in rows.split(|&b| matches!(b, b'\r' | b'\n')) {
            let row = row
                .iter()
                .position(|&b| !matches!(b, b' ' | b'\t'))
                .map_or(&[][..], |start| &row[start..]);
            if row.is_empty() || row.starts_with(b";") {
                continue;
            }
            let mut numbers = row
                .split(|&b| matches!(b, b',' | b' ' | b'\t'))
                .filter(|v| !v.is_empty());
            let Some(key) = numbers.next() else { continue };
            let key = integer2011(key);
            let mut values = [0; 8];
            for (destination, source) in values.iter_mut().zip(&mut numbers) {
                *destination = integer2011(source);
            }
            output.insert(key, values);
        }
        Ok(Self {
            table,
            rows: output,
        })
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct DbLookup {
    pub table: i32,
    pub key: Expression,
    pub outputs: [(i32, i32); 8], // source column, destination LocalHolder register
}

impl DbLookup {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        Self::parse_with(raw, db_operand)
    }

    pub fn parse_with(raw: &[u8], operand: impl Fn(&[u8]) -> Result<Expression>) -> Result<Self> {
        let mut fields = raw.split(|&b| b == b',');
        let mut next = || fields.next().unwrap_or_default();
        let table = integer2011(next());
        let key = operand(next())?;
        let outputs = std::array::from_fn(|_| (integer2011(next()), integer2011(next())));
        Ok(Self {
            table,
            key,
            outputs,
        })
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct DbWrite {
    pub table: i32,
    pub key: Expression,
    pub values: Vec<(i32, Expression)>,
}
impl DbWrite {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        Self::parse_with(raw, db_operand)
    }

    pub fn parse_with(raw: &[u8], operand: impl Fn(&[u8]) -> Result<Expression>) -> Result<Self> {
        let mut fields = raw.split(|&b| b == b',');
        let mut next = || fields.next().unwrap_or_default();
        let table = integer2011(next());
        let key = operand(next())?;
        let mut values = Vec::with_capacity(8);
        for _ in 0..8 {
            values.push((integer2011(next()), operand(next())?));
        }
        Ok(Self { table, key, values })
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct EntryAssignment {
    index: i32,
    name: Box<[u8]>,
}
impl EntryAssignment {
    pub fn parse(count: i32, tail: &[u8]) -> Result<Self> {
        // splitParam(..., keep_tail=true) leaves the final field intact.
        let fields = crate::v1_services::args(tail, count, true)?;
        Ok(Self {
            index: integer2011(fields[0]),
            name: fields[1].into(),
        })
    }
}

#[derive(Default)]
pub(super) struct EntryNames([Box<[u8]>; 8]);
impl EntryNames {
    pub fn set(&mut self, assignment: EntryAssignment) {
        if let Some(slot) = self.0.get_mut(assignment.index as usize) {
            *slot = assignment.name;
        }
    }
    pub fn count(&self) -> usize {
        self.0.iter().filter(|n| !n.is_empty()).count()
    }
    /// 0xb17e0 emits the original name bytes to T207.
    pub fn checkpoint_text(&self) -> Vec<u8> {
        let mut text = vec![b'8'];
        for (i, name) in self.0.iter().enumerate() {
            text.extend([b',', b'0' + i as u8, b'.']);
            text.extend_from_slice(name);
        }
        text
    }
}

/// Native action250 processes more directives (A, @, resource/volume and
/// callbacks). Only this literal register subset can be prepared here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RnfWrite {
    LocalFlag(u8, bool),
    NormalAndSystem(i32, i32),
    System(i32, i32),
    Relative(i32, i32),
}

pub(super) fn native_literal(bytes: &[u8]) -> Result<i32> {
    let bytes = bytes.trim_ascii();
    if let Some(raw) = bytes.strip_prefix(b"0x") {
        return Ok(u32::from_str_radix(std::str::from_utf8(raw)?, 16)? as i32);
    }
    if let Some(raw) = bytes.strip_prefix(b"0b") {
        return Ok(u32::from_str_radix(std::str::from_utf8(raw)?, 2)? as i32);
    }
    number(bytes)
}

#[allow(dead_code)] // stock action250 remains unsupported until other directive families join
pub(super) fn parse_literal_rnf(bytes: &[u8]) -> Result<Vec<RnfWrite>> {
    ensure!(bytes.len() <= 65536, "oversized ExHibit 2011 settings file");
    let mut writes = Vec::new();
    for (line, raw) in bytes.split(|b| *b == b'\n').enumerate() {
        let raw = raw.trim_ascii();
        if raw.is_empty() || raw.starts_with(b";") {
            continue;
        }
        let pos = raw
            .iter()
            .position(|&b| b == b'=')
            .context("missing ExHibit settings assignment")?;
        let key = raw[..pos].trim_ascii();
        let Some((&tag, index)) = key.split_first() else {
            bail!("empty ExHibit settings key at line {}", line + 1);
        };
        let tag = tag.to_ascii_uppercase();
        ensure!(
            matches!(tag, b'F' | b'R' | b'S' | b'L'),
            "unsupported ExHibit 2011 RNF directive at line {}",
            line + 1
        );
        let index = native_literal(index)?;
        ensure!(index >= 0, "negative ExHibit settings index");
        // Native action250 splits the right-hand side at a tab. Extra fields
        // are supported here only as a semicolon comment, not an expression.
        let rhs = raw[pos + 1..].trim_ascii();
        let mut fields = rhs.splitn(2, |&b| b == b'\t');
        let value = native_literal(fields.next().unwrap_or_default())?;
        ensure!(
            fields
                .next()
                .is_none_or(|f| f.trim_ascii().starts_with(b";")),
            "unsupported ExHibit settings suffix at line {}",
            line + 1
        );
        let write = match tag {
            b'F' => {
                ensure!(index < 32, "ExHibit local flag index out of range");
                RnfWrite::LocalFlag(index as u8, value != 0)
            }
            b'R' => RnfWrite::NormalAndSystem(index, value),
            b'S' => RnfWrite::System(index, value),
            b'L' => RnfWrite::Relative(index, value),
            _ => unreachable!(),
        };
        writes.push(write);
    }
    Ok(writes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Action, Script,
        registers::{Value, assignment},
    };

    #[test]
    fn integer_table_opcodes_replace_query_and_preserve_missing_outputs() -> Result<()> {
        let mut data = b"\0DLR".to_vec();
        for n in [3u32, 12, 6] {
            data.extend(n.to_le_bytes());
        }
        let query = b"1,R7,0,9,1,10,7,11,9,12,-1,13,0,-1,-1,-1,-1,-1";
        for (op, slot, text) in [
            (0x3d, Some(0i32), b"3\t1000".as_slice()),
            (0x3d, Some(1i32), b";comment\n3\t101\t-7\r\n3\t09\t-2\r\n"),
            (0x3e, None, query.as_slice()),
            (0x3d, Some(1i32), b"4,17"),
            (0x3e, None, query.as_slice()),
            (
                0x3e,
                None,
                b"0,R7,0,14,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1",
            ),
        ] {
            data.extend([op, 0, u8::from(slot.is_some()), 1]);
            if let Some(slot) = slot {
                data.extend(slot.to_le_bytes());
            }
            data.extend(text);
            data.push(0);
        }
        let mut script = Script::new(data, 3)?;
        let mut regs = crate::native_state_tests::fresh(3)?;
        regs.set_normal(7, 3)?;
        regs.set_normal(9, 42)?;
        regs.set_normal(12, 1)?;
        regs.set_normal(13, 1)?;
        for i in 0..6 {
            let Action::Change(change) = script.next()? else {
                bail!("expected Midori change");
            };
            // A missing row must preserve the value most recently set by
            // later script, not write a zero-filled default.
            if i == 4 {
                regs.change(assignment(6, b"0,9,0,45,*,")?)?;
            }
            regs.change(change)?;
            if i == 2 {
                assert_eq!(regs.read(Value::Local(9))?, 9);
                assert_eq!(regs.read(Value::Local(10))?, -2);
                assert_eq!(regs.read(Value::Local(11))?, 0);
                assert_eq!(regs.read(Value::Local(12))?, 0);
                assert_eq!(regs.read(Value::Local(13))?, 0);
            }
        }
        assert_eq!(regs.read(Value::Local(9))?, 45);
        assert_eq!(regs.read(Value::Local(14))?, 1000);
        assert_eq!(regs.read(Value::Local(0))?, 0);
        assert_eq!(script.remaining, 0);
        Ok(())
    }

    #[test]
    fn integer_table_writes_evaluate_pairs_and_update_only_existing_rows() -> Result<()> {
        let records = [
            (
                0x3d,
                Some(2_i32),
                b";ignored\r300,4,5,bad,7,8,9,10,11,999\r301,77".as_slice(),
            ),
            (0x3f, None, b"2,R7 3 +,1,R8 2 *,12,79,3,15,1,17,-1,0,0,999"),
            (0x3f, None, b"2,302,0,800,-1,0"),
            (0x3f, None, b"9,300,0,800,-1,0"),
            (0x3d, Some(-1), b"300,900"),
            (0x3e, None, b"2,R7 3 +,0,6,1,7,2,8,3,9,7,10"),
            (0x3e, None, b"2,302,0,6"),
            (0x3e, None, b"9,300,0,6"),
        ];
        let mut data = b"\0DLR".to_vec();
        for n in [3u32, 12, records.len() as u32] {
            data.extend(n.to_le_bytes());
        }
        for (op, table, fields) in records {
            data.extend([op, 0, u8::from(table.is_some()), 1]);
            if let Some(table) = table {
                data.extend(table.to_le_bytes());
            }
            data.extend(fields);
            data.push(0);
        }
        let mut script = Script::new(data, 3)?;
        let mut regs = crate::native_state_tests::fresh(3)?;
        regs.set_normal(7, 297)?;
        regs.set_normal(8, 5)?;
        for _ in records {
            let Action::Change(change) = script.next()? else {
                bail!("expected table change")
            };
            regs.change(change)?;
        }
        for (reg, expected) in [(6, 4), (7, 17), (8, 0), (9, 15), (10, 11), (0, 4)] {
            assert_eq!(regs.read(Value::Local(reg))?, expected, "R{reg}");
        }
        Ok(())
    }

    #[test]
    fn unsupported_integer_table_expression_retains_script_pc() -> Result<()> {
        for (op, raw) in [
            (0x3e, b"1,@unsupported(1 7),0,9".as_slice()),
            (0x3f, b"1,3,0,@unsupported(1 7)"),
        ] {
            let mut data = b"\0DLR".to_vec();
            for n in [3u32, 12, 1] {
                data.extend(n.to_le_bytes());
            }
            data.extend([op, 0, 0, 1]);
            data.extend(raw);
            data.push(0);
            let mut script = Script::new(data, 3)?;
            assert!(script.next().is_err(), "accepted op {op:#04x} {raw:?}");
            assert_eq!((script.pc, script.remaining), (16, 1));
        }
        Ok(())
    }

    #[test]
    fn entry_name_dispatch_keeps_other_slots_and_serializes_native_order() -> Result<()> {
        let mut data = b"\0DLR".to_vec();
        for n in [3u32, 12, 3] {
            data.extend(n.to_le_bytes());
        }
        for text in [
            b"30,-1,2,0,menu\0".as_slice(),
            b"30,-1,2,1,extra\0",
            b"30,-1,2,1,replay\0",
        ] {
            data.extend([0x0b, 0, 0, 1]);
            data.extend(text);
        }
        let mut script = Script::new(data, 3)?;
        let mut entries = EntryNames::default();
        for _ in 0..3 {
            let Action::MidoriEntryName(write) = script.next()? else {
                bail!("entry action form");
            };
            entries.set(write);
        }
        assert_eq!(
            entries.checkpoint_text(),
            b"8,0.menu,1.replay,2.,3.,4.,5.,6.,7."
        );
        assert_eq!(entries.count(), 2);
        entries.set(EntryAssignment::parse(2, b"0,")?);
        assert_eq!(entries.count(), 1);
        entries.set(EntryAssignment::parse(2, b"8,ignored")?);
        assert_eq!(entries.count(), 1);
        entries.set(EntryAssignment::parse(2, b"1,\x82\xa0,verbatim")?);
        assert_eq!(
            entries.checkpoint_text(),
            b"8,0.,1.\x82\xa0,verbatim,2.,3.,4.,5.,6.,7."
        );
        Ok(())
    }

    #[test]
    fn rnf_preserves_local_override_and_mirrors_r_values_to_system_atomically() -> Result<()> {
        let mut regs = crate::native_state_tests::fresh(3)?;
        regs.change(assignment(6, b"0,62,0,9,*,")?)?;
        let parsed = parse_literal_rnf(
            b"; generated fixture\r\nR62=0x20\t; mirror\r\nS63=3\r\nL6=0b101\r\nF25=1\r\nF2=0\r\n",
        )?;
        regs.apply_midori_rnf_literals(&parsed)?;
        assert_eq!(regs.read(Value::Local(62))?, 9);
        assert_eq!(regs.read(Value::System(62))?, 32);
        assert_eq!(regs.read(Value::System(63))?, 3);
        let base = regs.read(Value::System(15))?;
        assert_eq!(regs.read(Value::System(base + 6))?, 5);
        assert_eq!(regs.read(Value::Flag(25))?, 1);
        assert_eq!(regs.read(Value::Flag(2))?, 0);
        let bad = parse_literal_rnf(b"S63=127\nL9000=20\n")?;
        assert!(regs.apply_midori_rnf_literals(&bad).is_err());
        assert_eq!(regs.read(Value::System(63))?, 3);
        assert!(parse_literal_rnf(b"F1=1\n@313=2,30,100\n").is_err());
        assert!(parse_literal_rnf(b"S63=2+3\n").is_err());
        assert!(
            regs.apply_midori_rnf_literals(&parse_literal_rnf(b"S15=2\n")?)
                .is_err()
        );
        Ok(())
    }
}
