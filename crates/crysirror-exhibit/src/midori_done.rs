// SPDX-License-Identifier: MPL-2.0
//! 2011 cmdSetDoneFlag (0xb4880) and cmdExHIBIT case 39 (0x13d22f).
use super::{Bank, Registers};
use crate::native_numbers::integer2011 as integer;
use anyhow::{Context, Result, ensure};

#[derive(Debug, PartialEq)]
enum Target {
    Counter(Bank),
    ReadHistory,
}

#[derive(Debug, PartialEq)]
struct Span {
    first: i32,
    last: i32,
    parenthesized: bool,
}

#[derive(Debug, PartialEq)]
pub(crate) struct DoneFlags {
    target: Option<Target>,
    set: bool,
    spans: Vec<Span>,
    scene_single_updates_history: bool,
}

// FCString::next (0x14f510) skips leading delimiters, copies a field
// and advances past one delimiter. All delimiters here are ASCII below
// the valid CP932 trail-byte range.
fn next<'a>(text: &mut &'a [u8], delimiters: &[u8]) -> &'a [u8] {
    let first = text
        .iter()
        .position(|b| !delimiters.contains(b))
        .unwrap_or(text.len());
    let rest = &text[first..];
    let end = rest
        .iter()
        .position(|b| delimiters.contains(b))
        .unwrap_or(rest.len());
    *text = &rest[(end + usize::from(end < rest.len()))..];
    &rest[..end]
}

impl DoneFlags {
    pub(super) fn single(bank: Option<Bank>, id: i32, set: bool) -> Self {
        Self {
            target: Some(bank.map_or(Target::ReadHistory, Target::Counter)),
            set,
            spans: vec![Span { first: id, last: id, parenthesized: false }],
            scene_single_updates_history: false,
        }
    }

    pub(crate) fn parse(raw: &[u8]) -> Self {
        let mut fields = raw.split(|&b| b == b',');
        Self::from_fields(
            fields.next().unwrap_or_default(),
            fields.next().unwrap_or_default(),
            fields.next().unwrap_or_default(),
            true,
        )
    }

    pub(crate) fn service(count: i32, tail: &[u8]) -> Result<Self> {
        let args = crate::v1_services::args(tail, count, false)?;
        Ok(Self::from_fields(args[0], args[1], args[2], false))
    }

    fn from_fields(bank: &[u8], setting: &[u8], mut list: &[u8], opcode: bool) -> Self {
        let target = match integer(bank) {
            0 => Some(Target::Counter(Bank::Cg)),
            1 => Some(Target::Counter(Bank::Sound)),
            2 => Some(Target::Counter(Bank::Scene)),
            3 if opcode => Some(Target::ReadHistory),
            _ => None,
        };
        let mut spans = Vec::new();
        while !list.is_empty() {
            let mut field = next(&mut list, b".");
            if field.is_empty() {
                break;
            }
            let parenthesized = field.first() == Some(&b'(');
            let (first, last) = if parenthesized {
                let start = field
                    .iter()
                    .position(|b| !b"( ".contains(b))
                    .unwrap_or(field.len());
                field = &field[start..];
                (
                    integer(next(&mut field, b"- ")),
                    integer(next(&mut field, b" )")),
                )
            } else {
                let id = integer(field);
                (id, id)
            };
            spans.push(Span {
                first,
                last,
                parenthesized,
            });
        }
        Self {
            target,
            set: integer(setting) != 0,
            spans,
            scene_single_updates_history: opcode,
        }
    }

    pub(crate) fn apply(self, registers: &mut Registers) -> Result<()> {
        let Some(target) = self.target else {
            return Ok(());
        };
        let l = registers.limits.context("missing 2011 experience banks")?;
        let capacity = match target {
            Target::Counter(Bank::Cg) => l.cg,
            Target::Counter(Bank::Sound) => l.sound,
            Target::Counter(Bank::Scene) => l.scene,
            Target::ReadHistory => l.messages,
        } as i32;
        let indices = |s: &Span| s.first.max(0)..=s.last.min(capacity - 1);
        let steps: u64 = self
            .spans
            .iter()
            .map(|s| {
                let range = indices(s);
                (i64::from(*range.end()) - i64::from(*range.start()) + 1).max(0) as u64
            })
            .sum();
        ensure!(
            steps <= 16_000_000,
            "ExHibit experience list exceeds operation budget"
        );
        for span in &self.spans {
            if let Target::Counter(bank) = target {
                for id in indices(span) {
                    if self.set {
                        registers.increment_experience(bank, id, capacity as usize);
                    } else {
                        let entries = &mut registers.seen[bank as usize];
                        if entries.get(&id).is_some_and(|n| *n < 0x7fff_ff00) {
                            entries.remove(&id);
                        }
                    }
                }
            }
            // The scalar scene arm of cmdSetDoneFlag jumps into the message
            // arm even if the scene ID is out of bounds. Ranges and service 39 do not.
            if matches!(target, Target::ReadHistory)
                || (matches!(target, Target::Counter(Bank::Scene))
                    && self.scene_single_updates_history
                    && !span.parenthesized)
            {
                for id in span.first.max(0)..=span.last.min(l.messages as i32 - 1) {
                    if self.set {
                        registers.mark_message_in(id, l.messages);
                    } else if let Some(bits) = registers.messages.get_mut(&(id as usize / 32)) {
                        *bits &= !(1u32 << ((id as usize % 32) ^ 7));
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registers::{Limits, Value};
    use crate::{Action, Script};

    fn registers() -> Result<Registers> {
        Registers::new(Some(Limits {
            regs: 32,
            sys: 64,
            cg: 5,
            scene: 5,
            sound: 5,
            strings: 2,
            locals: 8,
            characters: 1,
            messages: 64,
        }))
    }
    fn execute(regs: &mut Registers, op: u16, text: &[u8]) -> Result<()> {
        let mut rld = b"\0DLR".to_vec();
        for n in [3_u32, 12, 1, 0x0100_0000 | u32::from(op)] {
            rld.extend(n.to_le_bytes());
        }
        rld.extend(text);
        rld.push(0);
        let Action::Change(change) = Script::new(rld, 3)?.next()? else {
            anyhow::bail!("expected register change");
        };
        regs.change(change)
    }

    #[test]
    fn script_done_distinguishes_scalar_scene_and_range_history() -> Result<()> {
        let mut regs = registers()?;
        execute(&mut regs, 0xbd, b"2,1,1.(2-4).2.7")?;
        assert_eq!(regs.seen[Bank::Scene as usize][&2], 2);
        assert_eq!(regs.read(Value::Seen(Bank::Scene, 4))?, 1);
        assert_eq!(regs.messages[&0], (1 << 6) | (1 << 5) | 1);
        execute(&mut regs, 0xbd, b"2,0,(1-4).7")?;
        assert_eq!(regs.read(Value::Seen(Bank::Scene, 2))?, 0);
        assert_eq!(regs.messages[&0], (1 << 6) | (1 << 5));
        // A space before the dash terminates the first FCString field;
        // the second field then starts with a lone minus and converts to zero.
        execute(&mut regs, 0xbd, b"3,1,( 3 - 12 )")?;
        assert_eq!(regs.messages[&0], 0x60);
        execute(&mut regs, 0xbd, b"3,1,( 3- 12 ).(60-2147483646)")?;
        assert_eq!(regs.messages[&0], 0xf81f | 0x60);
        assert_eq!(regs.messages[&1], 0x0f00_0000);
        execute(&mut regs, 0xbd, b"3,0,(0-2147483646)")?;
        assert_eq!(regs.messages.values().copied().sum::<u32>(), 0);
        Ok(())
    }

    #[test]
    fn services_respect_banks_frozen_counts_and_relative_system_index() -> Result<()> {
        let mut regs = registers()?;
        regs.set_normal(3, 88)?;
        execute(&mut regs, 0x0b, b"33,3,1,0x4")?;
        assert_eq!(regs.read(Value::Seen(Bank::Cg, 4))?, 1);
        regs.seen[Bank::Cg as usize].insert(2, 0x7fff_ff00);
        execute(&mut regs, 0x0b, b"39,3,3,0,0,(2-4)")?;
        assert_eq!(regs.seen[Bank::Cg as usize][&2], 0x7fff_ff00);
        assert_eq!(regs.read(Value::Seen(Bank::Cg, 4))?, 0);
        execute(&mut regs, 0x0b, b"39,3,3,2,1,2")?;
        execute(&mut regs, 0x0b, b"39,3,3,3,1,(0-32)")?;
        assert_eq!(regs.read(Value::Seen(Bank::Scene, 2))?, 1);
        assert!(regs.messages.is_empty());
        regs.set_system(15, 20)?;
        execute(&mut regs, 0x0b, b"48,3,0,-9,extra")?;
        assert_eq!(regs.read(Value::System(25))?, -9);
        assert_eq!(regs.read(Value::Local(3))?, 88);
        Ok(())
    }

    #[test]
    fn script_title_service_copies_literal_or_live_text_without_writing_result() -> Result<()> {
        let mut regs = registers()?;
        regs.set_normal(3, 12)?;
        regs.strings.insert(0, b"fallback".to_vec());
        execute(&mut regs, 0x0b, b"47,3,2,0,\x82\xa0,trailing")?;
        assert_eq!(regs.text(1)?, b"\x82\xa0");
        execute(&mut regs, 0x0b, b"47,3,2,1,1")?;
        assert_eq!(regs.text(1)?, b"\x82\xa0");
        execute(&mut regs, 0x0b, b"47,3,2,1,-2")?;
        assert_eq!(regs.text(1)?, b"fallback");
        execute(&mut regs, 0x0b, b"47,3,1,0,ignored")?;
        assert_eq!(regs.text(1)?, b"");
        assert_eq!(regs.read(Value::Local(3))?, 12);
        Ok(())
    }
}
