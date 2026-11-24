// SPDX-License-Identifier: MPL-2.0
//! Scoped Midori 2011 script-owned state and the literal subset of RNF.
use crate::{branch::number, registers::Value};
use anyhow::{Context, Result, bail, ensure};
use std::collections::BTreeMap;

#[derive(Debug, PartialEq)]
pub(super) struct DbDefinition {
    pub table: usize,
    pub rows: BTreeMap<i32, [i32; 8]>,
}

impl DbDefinition {
    pub fn parse(table: i32, rows: &[u8]) -> Result<Self> {
        let table = db_index(table)?;
        ensure!(rows.len() <= 65536, "oversized ExHibit integer table");
        let mut output = BTreeMap::new();
        for row in rows.split(|&b| b == b'\n') {
            let row = row.trim_ascii();
            if row.is_empty() || row.starts_with(b";") {
                continue;
            }
            let mut numbers = row
                .split(|&b| matches!(b, b',' | b' ' | b'\t'))
                .filter(|v| !v.is_empty());
            let key = native_literal(numbers.next().context("missing ExHibit table row key")?)?;
            let mut values = [0; 8];
            for (destination, source) in values.iter_mut().zip(&mut numbers) {
                *destination = native_literal(source)?;
            }
            ensure!(
                numbers.next().is_none(),
                "unsupported ExHibit table row width"
            );
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
    pub table: usize,
    pub key: Value,
    pub outputs: [(i32, i32); 8], // source column, destination LocalHolder register
}

impl DbLookup {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        let mut fields = raw.split(|&b| b == b',');
        let table = db_index(native_literal(fields.next().context("missing table")?)?)?;
        let raw_key = fields.next().context("missing lookup key")?.trim_ascii();
        let key = match raw_key.split_first() {
            Some((b'R' | b'r', index)) => {
                let index = number(index)?;
                ensure!(index >= 0, "negative ExHibit lookup register");
                Value::Local(index)
            }
            _ => Value::Immediate(native_literal(raw_key)?),
        };
        let mut outputs = [(0, 0); 8];
        for (column, destination) in &mut outputs {
            *column = native_literal(fields.next().context("missing lookup column")?)?;
            *destination = native_literal(fields.next().context("missing lookup register")?)?;
        }
        ensure!(fields.next().is_none(), "extra ExHibit lookup argument");
        Ok(Self {
            table,
            key,
            outputs,
        })
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct SceneDone(pub i32);

impl SceneDone {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        // The observed plain-item case (not the parenthesized list/range case):
        // kind 2, set; native increments scene and falls through to message.
        let fields: Vec<_> = raw.splitn(4, |&b| b == b',').collect();
        ensure!(
            fields.len() == 3 && fields[..2] == [b"2", b"1"],
            "unsupported ExHibit 2011 done flag"
        );
        Ok(Self(number(fields[2])?))
    }
}

fn db_index(table: i32) -> Result<usize> {
    let table = usize::try_from(table)?;
    ensure!(table < 8, "unsupported ExHibit integer table index");
    Ok(table)
}

#[derive(Debug, PartialEq)]
pub(super) struct EntryAssignment {
    index: usize,
    name: Box<str>,
}
impl EntryAssignment {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        let fields: Vec<_> = raw.splitn(6, |b| *b == b',').collect();
        ensure!(
            fields.len() == 5 && fields[..3] == [b"30", b"-1", b"2".as_slice()],
            "unsupported ExHibit 2011 command form"
        );
        let index = usize::try_from(number(fields[3])?)?;
        ensure!(index < 8, "unsupported ExHibit 2011 entry index");
        let name = std::str::from_utf8(fields[4])?;
        // Native copies this literal into the table. Restrict the observed
        // ordinary scenario-name form; it must not become an ambient path.
        if !name.is_empty() {
            crate::script_path(name)?;
        }
        Ok(Self {
            index,
            name: name.into(),
        })
    }
}

#[derive(Default)]
pub(super) struct EntryNames([Box<str>; 8]);
impl EntryNames {
    pub fn set(&mut self, assignment: EntryAssignment) {
        self.0[assignment.index] = assignment.name;
    }
    pub fn count(&self) -> usize {
        self.0.iter().filter(|n| !n.is_empty()).count()
    }
    /// 0xb17e0, for eventual capture to T207, not an original save.
    pub fn checkpoint_text(&self) -> Vec<u8> {
        let mut text = String::from("8");
        for (i, name) in self.0.iter().enumerate() {
            use std::fmt::Write;
            let _ = write!(&mut text, ",{i}.{name}");
        }
        text.into_bytes()
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

fn native_literal(bytes: &[u8]) -> Result<i32> {
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
    fn unfamiliar_integer_table_and_scene_flag_forms_retain_script_pc() -> Result<()> {
        for (op, index, raw) in [
            (0x3d, Some(8i32), b"2\t7".as_slice()),
            (0x3d, Some(1), b"2\t7\n3\tunsupported"),
            (0x3e, None, b"1,R7,0,9"),
            (0xbd, None, b"2,1,(1-2)"),
            (0xbd, None, b"2,0,1"),
        ] {
            let mut data = b"\0DLR".to_vec();
            for n in [3u32, 12, 1] {
                data.extend(n.to_le_bytes());
            }
            data.extend([op, 0, u8::from(index.is_some()), 1]);
            if let Some(index) = index {
                data.extend(index.to_le_bytes());
            }
            data.extend(raw);
            data.push(0);
            let mut script = Script::new(data, 3)?;
            assert!(script.next().is_err());
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
        entries.set(EntryAssignment::parse(b"30,-1,2,0,")?);
        assert_eq!(entries.count(), 1);
        assert!(EntryAssignment::parse(b"30,-1,2,8,escape").is_err());
        assert!(EntryAssignment::parse(b"30,-1,2,1,../outside").is_err());
        assert!(EntryAssignment::parse(b"30,0,2,1,name").is_err());
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
