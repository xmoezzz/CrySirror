// SPDX-License-Identifier: MPL-2.0
//! Vol. 1/2 liteSetReg(CMD_DATA&,LocalHolder&) at 0x1d4520 / 0x1d4aa0.
//! These residents have separate operand and condition grammars; this path
//! handles their numeric literals, scalar registers and one scalar comparison.
use crate::{
    Action, cstring,
    registers::{Registers, Value},
    word,
};
use anyhow::{Context, Result, bail, ensure};

/// liteSetReg dispatches 0x0a to text registers in both residents. The
/// old numeric form has no clear case, even though CSV mode 10 clears.
pub(super) fn text_assignment(
    bytes: &[u8],
    start: usize,
    mut end: usize,
    header: u32,
) -> Result<(Action, usize)> {
    use crate::native_numbers::integer2011;
    use crate::registers::{Change, TextValue};

    let count = ((header >> 24) & 15) as usize;
    ensure!(
        count <= 10 && ((header >> 16) & 255) <= 21,
        "invalid late text assignment fields"
    );
    let mut strings = [&[][..]; 2];
    for i in 0..count {
        let (field, next) = cstring(bytes, end)?;
        if let Some(slot) = strings.get_mut(i) {
            *slot = field;
        }
        end = next;
    }
    let csv = header >> 28 == 1;
    let (dest, mode, raw, condition, replacement) = if csv {
        let mut fields = strings[0].split(|&b| b == b',').skip(1);
        let mut field = || fields.next().unwrap_or_default();
        (
            integer2011(field()),
            integer2011(field()),
            field(),
            field(),
            field(),
        )
    } else {
        ensure!(
            (header >> 16) as u8 >= 3,
            "missing late text assignment words"
        );
        (
            word(bytes, start + 8)? as i32,
            word(bytes, start + 12)? as i32,
            strings[0],
            &[][..],
            strings[1],
        )
    };
    let mode = mode & 255;
    // LCMapStringA transformations require Japanese NLS conventions.
    ensure!(
        !matches!(mode, 4..=9),
        "unsupported late text-register NLS mode {mode}"
    );
    if mode > 10 || (!csv && mode == 10) {
        return Ok((Action::NativeMethodReturn, end));
    }
    let when = crate::later_flow::text_condition(condition)?;
    let source = match raw.split_first() {
        Some((b'"', tail)) => TextValue::Literal(
            tail.split(|&b| b == b'"')
                .next()
                .unwrap_or_default()
                .to_vec(),
        ),
        Some((&prefix @ (b'R' | b'S' | b'T'), tail)) if integer2011(tail) > 0 => {
            let index = integer2011(tail);
            match prefix {
                b'R' => TextValue::Global(index),
                b'S' => TextValue::System(index),
                _ => TextValue::MidoriText(index),
            }
        }
        _ => TextValue::Literal(raw.to_vec()),
    };
    Ok((
        Action::Change(Change::TextAssign {
            dest,
            mode,
            source,
            replacement: replacement.to_vec(),
            when,
        }),
        end,
    ))
}

#[derive(Debug, PartialEq)]
pub(super) struct Assignment {
    system: bool,
    index: i32,
    mode: u8,
    rhs: Value,
    comparison: Option<(Value, u8, Value)>,
    mirror: bool,
}

fn decimal(raw: &[u8]) -> Result<i32> {
    let s = std::str::from_utf8(raw)?.trim_matches([' ', '\t']);
    Ok(s.parse()
        .context("unverified late ExHibit integer operand")?)
}

pub(super) fn scalar(raw: &[u8]) -> Result<Value> {
    if raw.is_empty() {
        return Ok(Value::Immediate(0));
    }
    let (prefix, digits) = raw.split_first().expect("nonempty field");
    match prefix.to_ascii_uppercase() {
        b'R' => Ok(Value::Local(decimal(digits)?)),
        b'S' => Ok(Value::RelativeSystem(decimal(digits)?)),
        b'L' => Ok(Value::RelativeSystem(
            decimal(digits)?.wrapping_add(1_000_000),
        )),
        _ => Ok(Value::Immediate(decimal(raw)?)),
    }
}

fn comparison(raw: &[u8]) -> Result<Option<(Value, u8, Value)>> {
    if raw.is_empty() || raw == b"*" {
        return Ok(None);
    }
    // liteCheckConditionSub pair at 0x1cd1f0 / 0x1cd770. Function calls,
    // string comparisons and compound logical groups use additional paths.
    let pos = raw
        .windows(2)
        .position(|p| p[0] == 0x81 && (0x81..=0x86).contains(&p[1]))
        .context("unverified late ExHibit assignment condition")?;
    Ok(Some((
        scalar(&raw[..pos])?,
        raw[pos + 1],
        scalar(&raw[pos + 2..])?,
    )))
}

impl Assignment {
    pub(super) fn decode(
        bytes: &[u8],
        start: usize,
        operands_end: usize,
        header: u32,
    ) -> Result<(Action, usize)> {
        let system = header as u16 == 8;
        let (raw, end) = cstring(bytes, operands_end)?;
        let (index, flags, rhs, when, mirror) = match header >> 16 {
            0x1100 => {
                let mut args = raw.split(|b| *b == b',');
                let mut field = || args.next().unwrap_or_default();
                let _ignored = field();
                let index = field();
                let mode = field();
                let rhs = field();
                let when = field();
                let local_flags = field();
                let n = |s: &[u8]| if s.is_empty() { Ok(0) } else { decimal(s) };
                let mirror = !system && n(local_flags)? & 1 != 0;
                (n(index)?, n(mode)?, scalar(rhs)?, comparison(when)?, mirror)
            }
            // Legacy numeric-record form. Word zero is unused. The original
            // has separate RHS register lookup rules in this branch.
            0x0103 => {
                let index = word(bytes, start + 8)? as i32;
                let flags = word(bytes, start + 12)? as i32;
                ensure!(
                    (0..=4).contains(&(if system { flags & 255 } else { flags })),
                    "unverified late binary assignment mode"
                );
                (index, flags, Value::Immediate(decimal(raw)?), None, false)
            }
            _ => bail!("unsupported late ExHibit assignment encoding {header:#x}"),
        };
        // Bit 8 in a system-register operation calls flushSysreg
        // (0xc9040/0xc90e0). Admit the non-persistent path alone.
        ensure!(
            !system || flags & 0x100 == 0,
            "late ExHibit assignment requires native system-register persistence"
        );
        ensure!(
            index >= 0,
            "unverified negative late ExHibit assignment destination"
        );
        Ok((
            Action::LateAssign(Self {
                system,
                index,
                mode: flags as u8,
                rhs,
                comparison: when,
                mirror,
            }),
            end,
        ))
    }

    pub(super) fn apply(&self, regs: &mut Registers) -> Result<()> {
        if let Some((left, op, right)) = self.comparison {
            let a = regs.read(left)?;
            let b = regs.read(right)?;
            let pass = match op {
                0x81 => a == b,
                0x82 => a != b,
                0x83 => a < b,
                0x84 => a > b,
                0x85 => a <= b,
                0x86 => a >= b,
                _ => unreachable!(),
            };
            if !pass {
                return Ok(());
            }
        }
        let operand = regs.read(self.rhs)?;
        let index = if self.system && self.index >= 1_000_000 {
            self.index
                .wrapping_sub(1_000_000)
                .wrapping_add(regs.read(Value::System(15))?)
        } else {
            self.index
        };
        let old = regs.read(if self.system {
            Value::System(index)
        } else {
            Value::Local(index)
        })?;
        let next = match self.mode {
            0 => Some(operand),
            1 => Some(old.wrapping_add(operand)),
            2 => Some(old.wrapping_mul(operand)),
            3 if operand != 0 => Some(
                old.checked_div(operand)
                    .context("late ExHibit division overflow")?,
            ),
            4 if operand > 0 => Some(old % operand),
            5 => Some(old & operand),
            6 => Some(old | operand),
            7 => Some(old ^ operand),
            10 => Some(0),
            // The native zero-divisor diagnostic and unsupported mode arms
            // leave the operand alone and still reach the normal-register mirror.
            _ => None,
        };
        if let Some(value) = next {
            if self.system {
                regs.set_system(index, value)?;
            } else {
                regs.set_local(index, value)?;
            }
        }
        if self.mirror {
            regs.set_normal(index, next.unwrap_or(old))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Script,
        branch::Condition,
        profile::Resident,
        registers::{Change, TextValue},
    };

    fn script(resident: Resident, op: u16, words: &[i32], flags: u8, raw: &[u8]) -> Result<Script> {
        let mut file = b"\0DLR".to_vec();
        for n in [
            3_u32,
            12,
            1,
            u32::from(op) | (words.len() as u32) << 16 | u32::from(flags) << 24,
        ] {
            file.extend(n.to_le_bytes());
        }
        for n in words {
            file.extend(n.to_le_bytes());
        }
        file.extend(raw);
        file.push(0);
        let mut script = Script::new(file, 3)?;
        script.resident = Some(resident);
        Ok(script)
    }

    fn step(regs: &mut Registers, mut script: Script) -> Result<()> {
        let Action::LateAssign(change) = script.next()? else {
            bail!("late numeric dispatch escaped")
        };
        change.apply(regs)?;
        ensure!(script.remaining == 0, "assignment did not advance");
        Ok(())
    }

    #[test]
    fn late_text_assignment_keeps_global_sources_and_both_record_forms_distinct() -> Result<()> {
        for profile in [Resident::LateVol1, Resident::LateVol2] {
            let mut regs = crate::native_state_tests::fresh(3)?;
            regs.set_normal(8, 31)?;
            regs.set_local(8, 112)?;
            let mut run = |words: &[i32], flags, raw: &[u8]| -> Result<Vec<u8>> {
                let mut s = script(profile, 10, words, flags, raw)?;
                match s.next()? {
                    Action::Change(c) => regs.change(c)?,
                    Action::NativeMethodReturn => {}
                    _ => bail!("unexpected late text-register action"),
                }
                assert_eq!(s.pc, s.bytes.len());
                Ok(regs.text(6)?.to_vec())
            };
            assert_eq!(run(&[], 0x11, b"*,6,0,R8,*,")?, b"31");
            assert_eq!(run(&[0, 6, 1], 1, b"\"x\"after")?, b"31x");
            assert_eq!(run(&[], 0x11, b"*,6,2,T6,R8\x81\x81112,")?, b"31x31x");
            assert_eq!(run(&[0, 6, 3], 2, b"x\0\x82\xa0")?, b"31\x82\xa031\x82\xa0");
            assert_eq!(run(&[0, 6, 10], 1, b"ignored")?, b"31\x82\xa031\x82\xa0");
            assert_eq!(
                run(&[], 0x11, b"*,6,10,ignored,false,")?,
                b"31\x82\xa031\x82\xa0"
            );
            assert_eq!(run(&[], 0x11, b"*,6,10,ignored,*,")?, b"");
            assert!(run(&[], 0x11, b"*,6,9,x,*,").is_err());
        }
        Ok(())
    }

    fn global(regs: &mut Registers, index: i32) -> Result<i32> {
        regs.change(Change::TextAssign {
            dest: 10,
            mode: 0,
            source: TextValue::Global(index),
            replacement: Vec::new(),
            when: Condition::text(b"*")?,
        })?;
        decimal(regs.text(10)?)
    }

    #[test]
    fn both_volumes_honor_local_mirror_bit_predicate_and_system_base() -> Result<()> {
        for resident in [Resident::LateVol1, Resident::LateVol2] {
            let mut regs = crate::native_state_tests::fresh(3)?;
            regs.set_normal(7, 100)?;
            regs.set_system(15, 82)?;
            let csv = |op, raw: &[u8]| script(resident, op, &[], 0x11, raw);
            step(&mut regs, csv(6, b"0,7,256,23,*,0")?)?;
            assert_eq!(regs.read(Value::Local(7))?, 23);
            assert_eq!(global(&mut regs, 7)?, 100);
            step(&mut regs, csv(6, b"0,7,1,R7,R7\x81\x8424,1")?)?;
            assert_eq!(regs.read(Value::Local(7))?, 23);
            assert_eq!(global(&mut regs, 7)?, 100);
            step(&mut regs, csv(6, b"0,7,1,R7,R7\x81\x8523,1")?)?;
            assert_eq!(global(&mut regs, 7)?, 46);
            step(&mut regs, csv(8, b"0,1000003,0,R7,*,1")?)?;
            assert_eq!(regs.read(Value::System(85))?, 46);
            step(&mut regs, csv(6, b"0,8,0,L3,,")?)?;
            assert_eq!(regs.read(Value::Local(8))?, 46);
            step(&mut regs, csv(6, b"0,8,3,0,*,1")?)?;
            assert_eq!(global(&mut regs, 8)?, 46);
        }
        Ok(())
    }

    #[test]
    fn binary_literal_modes_and_unverified_flush_are_profile_scoped() -> Result<()> {
        for resident in [Resident::LateVol1, Resident::LateVol2] {
            let mut regs = crate::native_state_tests::fresh(3)?;
            step(&mut regs, script(resident, 8, &[99, 32, 0], 1, b"-19")?)?;
            step(&mut regs, script(resident, 8, &[99, 32, 4], 1, b"5")?)?;
            assert_eq!(regs.read(Value::System(32))?, -4);
            step(&mut regs, script(resident, 6, &[99, 7, 0], 1, b"15")?)?;
            step(&mut regs, script(resident, 6, &[99, 7, 2], 1, b"-2")?)?;
            assert_eq!(regs.read(Value::Local(7))?, -30);
            for mut unsupported in [
                script(resident, 8, &[], 0x11, b"0,32,256,999,*,0")?,
                script(resident, 8, &[0, 32, 256], 1, b"999")?,
                script(resident, 6, &[], 0x11, b"0,7,0,@unknown(1),*,0")?,
                script(resident, 6, &[0, 7, 0], 1, b"R2")?,
            ] {
                assert!(unsupported.next().is_err());
                assert_eq!((unsupported.pc, unsupported.remaining), (16, 1));
            }
            assert_eq!(regs.read(Value::System(32))?, -4);
        }
        Ok(())
    }
}
