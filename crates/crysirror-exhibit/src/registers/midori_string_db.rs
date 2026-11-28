// SPDX-License-Identifier: MPL-2.0
//! RetouchSystem's eight 2011 string databases (0xd0–0xd2).
use super::{Bank, DoneFlags, Expression, Limits, Registers, Value, db_operand};
use crate::native_numbers::integer2011 as number;
use anyhow::{Context, Result, ensure};
use std::collections::BTreeMap;

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Table {
    rows: BTreeMap<i32, Vec<u8>>,
    bytes: usize,
}

impl Table {
    fn insert(&mut self, key: i32, row: Vec<u8>) -> Result<()> {
        let old = self.rows.get(&key);
        let bytes = self.bytes - old.map_or(0, Vec::len) + row.len();
        ensure!(
            bytes <= 8 << 20 && (old.is_some() || self.rows.len() < 65536),
            "ExHibit string database exceeds session budget"
        );
        self.rows.insert(key, row);
        self.bytes = bytes;
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum Command {
    Create(i32, Table),
    Access {
        table: i32,
        key: Expression,
        pairs: Vec<(i32, Vec<u8>)>,
        set: bool,
    },
}

// FCString::split with a tab as both skip-set and separator (0x14f6b0).
// Leading/consecutive tabs are skipped at each read. Exactly one terminal
// delimiter also appends an empty last field when the array has space.
fn columns(mut row: &[u8]) -> Vec<&[u8]> {
    let mut fields = Vec::new();
    while fields.len() < 16 {
        row = &row[row.iter().take_while(|&&b| b == b'\t').count()..];
        if row.is_empty() {
            break;
        }
        let Some(end) = row.iter().position(|&b| b == b'\t') else {
            fields.push(row);
            break;
        };
        fields.push(&row[..end]);
        row = &row[end + 1..];
        if row.is_empty() && fields.len() < 16 {
            fields.push(&[]);
            break;
        }
    }
    fields
}

impl Registers {
    pub(crate) fn first_transfer_route(
        &self,
        table: i32,
        condition_regs: &Registers,
    ) -> Result<Option<&[u8]>> {
        let Some(table) = self.string_tables.get(table as usize) else {
            return Ok(None);
        };
        for row in table.rows.values() {
            let fields = columns(row);
            let mut route = fields
                .get(1)
                .copied()
                .unwrap_or_default()
                .split(|&byte| byte == b',');
            let condition = route.next().unwrap_or_default().trim_ascii_start();
            let target = route.next().unwrap_or_default().trim_ascii_start();
            if crate::branch::Condition::for_abi(condition, 3)?.matches(condition_regs)? {
                return Ok(Some(target));
            }
        }
        Ok(None)
    }
}

impl Command {
    pub(crate) fn create(table: i32, data: &[u8]) -> Result<Self> {
        let mut out = Table::default();
        if (0..8).contains(&table) {
            // createStrDB (0x11e730) retains the complete text of each line.
            // The line reader skips leading horizontal whitespace.
            for line in data.split(|&b| matches!(b, b'\r' | b'\n')) {
                let line = &line[line
                    .iter()
                    .take_while(|&&b| matches!(b, b' ' | b'\t'))
                    .count()..];
                if line.is_empty() || line[0] == b';' {
                    continue;
                }
                let key = number(line.split(|&b| b == b'\t').next().unwrap_or_default());
                out.insert(key, line.to_vec())?;
            }
        }
        Ok(Self::Create(table, out))
    }

    pub(crate) fn access(raw: &[u8], set: bool) -> Result<Self> {
        let mut fields = raw.split(|&b| b == b',');
        let mut next = || fields.next().unwrap_or_default();
        let table = number(next());
        let key = db_operand(next())?;
        let pairs = (0..8).map(|_| (number(next()), next().to_vec())).collect();
        Ok(Self::Access {
            table,
            key,
            pairs,
            set,
        })
    }

    pub(crate) fn apply(self, regs: &mut Registers) -> Result<()> {
        match self {
            Self::Create(index, new) => {
                if let Some(table) = regs.string_tables.get_mut(index as usize) {
                    *table = new;
                }
            }
            Self::Access {
                table,
                key,
                pairs,
                set,
            } => {
                let key = key.eval(regs)?;
                let Some(table) = regs.string_tables.get_mut(table as usize) else {
                    return Ok(());
                };
                let old = table.rows.get(&key);
                if set {
                    let initial_key = key.to_string();
                    let mut fields =
                        old.map_or_else(|| vec![initial_key.as_bytes()], |row| columns(row));
                    let mut count = fields.len();
                    fields.resize(16, &[]);
                    for (col, text) in &pairs {
                        if let Some(cell) = (0..15).contains(col).then(|| *col as usize + 1) {
                            // 0x10cff6/0x10d060 use max(count, col+1), not
                            // col+2. The last extension is lost on rejoin.
                            count = count.max(cell);
                            fields[cell] = if text == b"*" { &[] } else { text };
                        }
                    }
                    table.insert(key, fields[..count.max(1)].join(&b'\t'))?;
                } else if let Some(row) = old.cloned() {
                    let fields = columns(&row);
                    for (column, destination) in pairs {
                        if let Ok(col) = usize::try_from(column)
                            && let Some(text) = fields.get(col + 1)
                        {
                            store(regs, &destination, text)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

// liteSetRegSub (0xc8400), reached for every in-range non-text column.
fn store(regs: &mut Registers, target: &[u8], text: &[u8]) -> Result<()> {
    let Some((&tag, tail)) = target.split_first() else {
        return Ok(());
    };
    let limits: Limits = regs
        .limits
        .context("missing ExHibit register configuration")?;
    let index = number(tail);
    let value = number(text);
    let set_bit = |old: i32, n: i32| {
        if value != 0 {
            old | (1 << (n & 31))
        } else {
            old & !(1 << (n & 31))
        }
    };
    match tag.to_ascii_uppercase() {
        b'T' => {
            regs.strings
                .insert(Registers::index(index, limits.strings), text.to_vec());
        }
        b'R' => regs.set_local(index, value)?,
        b'S' => regs.store(Value::RelativeSystem(index), value, limits),
        b'L' => regs.store(
            Value::RelativeSystem(index.wrapping_add(1_000_000)),
            value,
            limits,
        ),
        b'C' | b'F' => {
            let dest = if tag.eq_ignore_ascii_case(&b'C') {
                Value::System(23)
            } else {
                Value::RelativeSystem(1_000_004)
            };
            regs.store(dest, set_bit(regs.read(dest)?, index), limits);
        }
        b'B' if tail.first().is_some_and(|t| t.eq_ignore_ascii_case(&b'R')) => {
            let pair = &tail[1..];
            let register = number(pair);
            let skip = match pair.first() {
                Some(0x81..=0x9f | 0xe0..=0xfc) => 2,
                Some(_) => 1,
                None => 0,
            };
            let after_first = pair.get(skip..).unwrap_or_default();
            let bit = after_first
                .iter()
                .position(|&b| b == b'.')
                .map_or(0, |dot| {
                    let dots = &after_first[dot..];
                    number(&dots[dots.iter().take_while(|&&b| b == b'.').count()..])
                });
            if (0..32).contains(&bit) {
                regs.set_local(register, set_bit(regs.read(Value::Local(register))?, bit))?;
            }
        }
        b'M' if !tail.is_empty() => {
            let bank = match tail[0].to_ascii_uppercase() {
                b'G' => Some(Bank::Cg),
                b'M' => Some(Bank::Sound),
                b'S' => Some(Bank::Scene),
                b'V' => None,
                _ => return Ok(()),
            };
            DoneFlags::single(bank, number(&tail[1..]), value != 0).apply(regs)?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Script};
    fn execute(regs: &mut Registers, op: u16, table: Option<i32>, text: &[u8]) -> Result<()> {
        let mut rld = b"\0DLR".to_vec();
        for word in [
            3_u32,
            12,
            1,
            0x0100_0000 | ((table.is_some() as u32) << 16) | u32::from(op),
        ] {
            rld.extend(word.to_le_bytes());
        }
        if let Some(table) = table {
            rld.extend(table.to_le_bytes());
        }
        rld.extend(text);
        rld.push(0);
        let mut script = Script::new(rld, 3)?;
        let Action::Change(action) = script.next()? else {
            anyhow::bail!("missing string database dispatch")
        };
        regs.change(action)?;
        assert_eq!(script.remaining, 0);
        Ok(())
    }

    #[test]
    fn native_tables_keep_raw_cp932_and_apply_numeric_and_text_destinations_in_order() -> Result<()>
    {
        let mut regs = crate::native_state_tests::fresh(3)?;
        regs.set_normal(7, 209)?;
        execute(
            &mut regs,
            0xd0,
            Some(2),
            b"  ;comment\r 210\told\n210\t0x11\t\t\x82\xa0\t-5tail\r1\tunrelated",
        )?;
        execute(
            &mut regs,
            0xd1,
            None,
            b"2,R7 1 +,0,R8,1,T3,2,S38,0,C1,0,F2,0,BR9.3,0,MS1,0,MV33",
        )?;
        assert_eq!(regs.read(Value::Local(8))?, 17);
        assert_eq!(regs.text(3)?, b"\x82\xa0");
        assert_eq!(regs.read(Value::System(38))?, -5);
        assert_eq!(regs.read(Value::SystemFlag(1))?, 1);
        assert_eq!(regs.read(Value::Flag(2))?, 1);
        assert_eq!(regs.read(Value::Local(9))?, 8);
        assert_eq!(regs.read(Value::Seen(Bank::Scene, 1))?, 1);
        assert_eq!(regs.messages.get(&0).copied().unwrap_or_default(), 0);
        assert_eq!(regs.messages[&1], 1 << 6);
        execute(
            &mut regs,
            0xd1,
            None,
            b"2,210,1,R8,1,C1,1,F2,1,BR9.3,1,MS1,1,MV33,15,T3,-1,T3",
        )?;
        assert_eq!(regs.read(Value::Local(8))?, 0);
        assert_eq!(regs.read(Value::SystemFlag(1))?, 0);
        assert_eq!(regs.read(Value::Flag(2))?, 0);
        assert_eq!(regs.read(Value::Local(9))?, 0);
        assert_eq!(regs.read(Value::Seen(Bank::Scene, 1))?, 0);
        assert_eq!(regs.messages[&1], 0);
        assert_eq!(regs.text(3)?, b"\x82\xa0");
        execute(&mut regs, 0xd0, Some(2), b"211\tnew")?;
        execute(&mut regs, 0xd1, None, b"2,210,0,T3")?;
        assert_eq!(regs.text(3)?, b"\x82\xa0");
        Ok(())
    }

    #[test]
    fn native_sparse_writes_match_emitted_width_and_last_duplicate_wins() -> Result<()> {
        let mut regs = crate::native_state_tests::fresh(3)?;
        execute(&mut regs, 0xd0, Some(0), b"001\told\tsecond\tthird")?;
        execute(
            &mut regs,
            0xd2,
            None,
            b"0,1,0,first,1,*,0,changed,16,no,-1,,-1,,-1,,-1,",
        )?;
        assert_eq!(regs.string_tables[0].rows[&1], b"001\tchanged\t\tthird");
        execute(&mut regs, 0xd1, None, b"0,1,0,T2,1,T3,2,T4")?;
        assert_eq!(regs.text(2)?, b"changed");
        assert_eq!(regs.text(3)?, b"third");
        assert_eq!(regs.text(4)?, b"");
        execute(
            &mut regs,
            0xd2,
            None,
            b"0,700,0,one,1,two,2,omitted,-1,,-1,,-1,,-1,,-1,",
        )?;
        assert_eq!(regs.string_tables[0].rows[&700], b"700\tone\ttwo");
        execute(&mut regs, 0xd1, None, b"0,700,1,T4,2,T3")?;
        assert_eq!(regs.text(4)?, b"two");
        assert_eq!(regs.text(3)?, b"third");
        execute(&mut regs, 0xd2, None, b"9,700,0,ignored")?;
        execute(&mut regs, 0xd0, Some(8), b"700\tignored")?;
        assert_eq!(regs.string_tables[0].rows[&700], b"700\tone\ttwo");
        Ok(())
    }
}
