// SPDX-License-Identifier: MPL-2.0
//! Both late liteExec builds handle jump, call and return in their direct switch.
use crate::{
    Action, Registers, Script, Value, branch::Condition, cstring, late_numeric, native_numbers,
    registers::Change, word,
};
use anyhow::{Context, Result, bail, ensure};

// blockId2Index (0x159c90 / 0x159d60) compares the first matching
// explicit/implicit block or label, and falls back to record zero for ID zero.
pub(super) fn labels(
    bytes: &[u8],
    mut pos: crate::Position,
) -> Result<std::collections::BTreeMap<i32, crate::Position>> {
    let start = pos;
    let mut labels = std::collections::BTreeMap::new();
    let mut block = 0i32;
    while pos.remaining > 0 {
        let (header, first, next) = crate::branch::envelope(bytes, pos.pc)?;
        let words = (header >> 16) as u8;
        match header as u16 {
            5 => {
                let id = if header >= 0x1000_0000 {
                    native_numbers::integer2011(first.unwrap_or_default())
                } else {
                    ensure!(words > 0, "unverified late block without numeric ID");
                    word(bytes, pos.pc + 4)? as i32
                };
                block = if id == -1 { block.wrapping_add(1) } else { id };
                labels.entry(block).or_insert(pos);
            }
            0x2f => {
                ensure!(words > 0, "unverified late label without numeric ID");
                labels.entry(word(bytes, pos.pc + 4)? as i32).or_insert(pos);
            }
            _ => {}
        }
        pos = crate::Position {
            pc: next,
            remaining: pos.remaining - 1,
        };
    }
    labels.entry(0).or_insert(start);
    Ok(labels)
}

#[derive(Debug, PartialEq)]
pub(super) struct Flow {
    op: u16,
    target: i32,
    when: Condition,
    assignments: Vec<u8>,
}

// Only feed operands independently established for the two late calcExpression
// implementations to the shared conditional evaluator.
pub(super) fn text_condition(raw: &[u8]) -> Result<Condition> {
    if raw.is_empty() || raw == b"*" {
        return Condition::text(b"*");
    }
    for group in raw.split(|&b| b == b';').take(24) {
        for term in group.split(|&b| b == b'&') {
            let term = term.trim_ascii();
            if matches!(term, b"true" | b"false") {
                continue;
            }
            let pos = term
                .windows(2)
                .position(|op| op[0] == 0x81 && (0x81..=0x86).contains(&op[1]))
                .context("unverified late branch condition")?;
            late_numeric::scalar(&term[..pos])?;
            late_numeric::scalar(&term[pos + 2..])?;
        }
    }
    Condition::for_abi(raw, 3)
}

impl Flow {
    pub(super) fn decode(
        bytes: &[u8],
        start: usize,
        mut end: usize,
        header: u32,
    ) -> Result<(Action, usize)> {
        let op = header as u16;
        let mut flow = Self {
            op,
            target: 0,
            when: Condition::text(b"*")?,
            assignments: Vec::new(),
        };
        let flags = (header >> 24) as u8;
        let words = usize::from((header >> 16) as u8);
        if flags >> 4 == 0 {
            ensure!(
                words == 16 && flags == 5,
                "unverified late binary branch record {header:#010x}"
            );
            let values: Vec<i32> = (0..words)
                .map(|i| word(bytes, start + 4 + 4 * i).map(|v| v as i32))
                .collect::<Result<_>>()?;
            let mut strings = Vec::new();
            for clause in values[1..].chunks_exact(3) {
                let (field, next) = cstring(bytes, end)?;
                if matches!(clause[0], 0 | 1) && (0..=5).contains(&clause[2]) {
                    late_numeric::scalar(field)?;
                }
                strings.push(field);
                end = next;
            }
            flow.target = values[0];
            flow.when = Condition::binary(&values, &strings)?;
        } else {
            ensure!(flags & 15 == 1, "unverified late text branch fields");
            let (raw, next) = cstring(bytes, end)?;
            end = next;
            match op {
                0x14 => {
                    ensure!(words == 1, "missing late jump target");
                    flow.target = word(bytes, start + 4)? as i32;
                    flow.when = text_condition(raw)?;
                }
                0x16 => {
                    ensure!(words == 0, "unverified late call with numeric fields");
                    let mut fields = raw.splitn(3, |&b| b == b',');
                    flow.target = native_numbers::integer2011(fields.next().unwrap_or_default());
                    flow.when = text_condition(fields.next().unwrap_or_default())?;
                    flow.assignments = fields.next().unwrap_or_default().to_vec();
                }
                0x17 => {
                    ensure!(words == 0, "unverified late return with numeric fields");
                    let mut fields = raw.splitn(2, |&b| b == b',');
                    if native_numbers::integer2011(fields.next().unwrap_or_default()) & 1 != 0 {
                        // cmdReturn calls liteCheckConditionEngine directly. Its
                        // empty list has zero successful alternatives.
                        let field = fields.next().unwrap_or_default();
                        flow.when = if field.is_empty() {
                            text_condition(b"false")?
                        } else {
                            text_condition(field)?
                        };
                    }
                }
                _ => unreachable!(),
            }
        }
        Ok((Action::LateFlow(flow), end))
    }

    pub(super) fn apply(self, script: &mut Script, regs: &mut Registers) -> Result<()> {
        if !self.when.matches(regs)? {
            return Ok(());
        }
        // liteSetReg runs only for a taken textual call and after resolving its
        // callee. Ordinary call frames save the caller block and next record.
        let writes = if self.op == 0x16 && self.assignments != b"*" {
            scalar_writes(&self.assignments)?
        } else {
            Vec::new()
        };
        script.branch(self.op, self.target)?;
        for change in writes {
            regs.change(change)?;
        }
        Ok(())
    }
}

fn scalar_writes(raw: &[u8]) -> Result<Vec<Change>> {
    let mut writes = Vec::new();
    for assignment in raw.split(|&b| b == b';').take(32) {
        let assignment = assignment.trim_ascii();
        if assignment.is_empty() {
            continue;
        }
        let mut sides = assignment.splitn(2, |&b| b == b'=');
        let dest = sides.next().unwrap_or_default().trim_ascii();
        let source = sides
            .next()
            .context("missing late call assignment value")?
            .trim_ascii();
        let when = Condition::text(b"*")?;
        if let Some(index) = dest.strip_prefix(b"T") {
            let dest = std::str::from_utf8(index)?.parse::<i32>()?;
            let source = if let Some(source_index) = source.strip_prefix(b"T") {
                crate::registers::TextValue::Text(std::str::from_utf8(source_index)?.parse()?)
            } else if source.starts_with(b"\"") {
                // FCString skips leading quote delimiters, then copies the first
                // nonempty span up to the next quote (or end of string).
                ensure!(
                    !source.iter().any(u8::is_ascii_whitespace),
                    "unverified whitespace in quoted late call assignment"
                );
                let value = source
                    .split(|&b| b == b'"')
                    .find(|span| !span.is_empty())
                    .unwrap_or_default();
                crate::registers::TextValue::Literal(value.to_vec())
            } else {
                bail!("unverified late text assignment source");
            };
            writes.push(Change::TextAssign {
                dest,
                mode: 0,
                source,
                replacement: Vec::new(),
                when,
            });
        } else {
            let target = late_numeric::scalar(dest)?;
            if !matches!(target, Value::Local(_) | Value::RelativeSystem(_)) {
                bail!("unverified late call assignment destination");
            }
            writes.push(Change::Assign(
                target,
                0,
                late_numeric::scalar(source)?.into(),
                when,
            ));
        }
    }
    Ok(writes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Resident;

    fn record(id: u16, encoding: u8, words: &[i32], strings: &[&[u8]]) -> Vec<u8> {
        let mut bytes = id.to_le_bytes().to_vec();
        bytes.extend([words.len() as u8, encoding | strings.len() as u8]);
        for value in words {
            bytes.extend(value.to_le_bytes());
        }
        for value in strings {
            bytes.extend(*value);
            bytes.push(0);
        }
        bytes
    }

    fn script(records: &[Vec<u8>], resident: Resident) -> Result<Script> {
        let mut bytes = vec![0; 20];
        bytes[..4].copy_from_slice(b"\0DLR");
        bytes[4..8].copy_from_slice(&3u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&16u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&(records.len() as u32).to_le_bytes());
        for r in records {
            bytes.extend(r);
        }
        let mut s = Script::new(bytes, 3)?;
        s.resident = Some(resident);
        Ok(s)
    }

    fn execute(s: &mut Script, r: &mut Registers) -> Result<()> {
        let Action::LateFlow(cmd) = s.next()? else {
            bail!("expected native control-flow opcode")
        };
        cmd.apply(s, r)
    }

    #[test]
    fn late_flow_conditional_call_commits_scalar_arguments_in_order_and_returns() -> Result<()> {
        let records = [
            record(0x16, 0x10, &[], &[b"90,false,unreachable=invalid"]),
            record(
                0x16,
                0x10,
                &[],
                &[b"7,R3\x81\x815,R1=41; R2=R1; R1=9;T3=\"ef\";T4=T3"],
            ),
            record(0x14, 0x10, &[10], &[b"*"]),
            record(0x2f, 0, &[7], &[]),
            record(0x17, 0x10, &[], &[b"1,false"]),
            record(0x17, 0x10, &[], &[b"0,ignored"]),
            record(0x2f, 0, &[10], &[]),
        ];
        for resident in [Resident::LateVol1, Resident::LateVol2] {
            let mut s = script(&records, resident)?;
            let mut regs = crate::native_state_tests::fresh(3)?;
            regs.set_local(3, 5)?;
            s.block = 19;
            execute(&mut s, &mut regs)?;
            assert!(s.returns.is_empty());
            execute(&mut s, &mut regs)?;
            assert_eq!(s.returns.as_slice(), &[(2, 19)]);
            assert_eq!(regs.read(Value::Local(1))?, 9);
            assert_eq!(regs.read(Value::Local(2))?, 41);
            assert_eq!(regs.text(4)?, b"ef");
            assert_eq!(s.next()?, Action::NativeMethodReturn);
            s.block = 4;
            execute(&mut s, &mut regs)?;
            assert_eq!(s.returns.len(), 1);
            execute(&mut s, &mut regs)?;
            assert_eq!(s.block, 19);
            assert!(s.returns.is_empty());
            execute(&mut s, &mut regs)?;
            assert_eq!(s.remaining, 1);
            assert_eq!(s.next()?, Action::NativeMethodReturn);
        }
        Ok(())
    }

    #[test]
    fn late_flow_binary_jump_compares_all_active_slots() -> Result<()> {
        let values = [11, 0, 1, 3, 0, 2, 5, -1, 0, 0, -1, 0, 0, -1, 0, 0];
        let records = [
            record(0x14, 0, &values, &[b"10", b"7", b"", b"", b""]),
            record(0x2f, 0, &[9], &[]),
            record(0x2f, 0, &[11], &[]),
        ];
        for resident in [Resident::LateVol1, Resident::LateVol2] {
            for (first, second, remaining) in [(9, 5, 2), (10, 7, 2), (10, 6, 1)] {
                let mut s = script(&records, resident)?;
                let mut regs = crate::native_state_tests::fresh(3)?;
                regs.set_local(1, first)?;
                regs.set_local(2, second)?;
                execute(&mut s, &mut regs)?;
                assert_eq!(s.remaining, remaining);
            }
        }
        Ok(())
    }
}
