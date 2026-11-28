// SPDX-License-Identifier: MPL-2.0
//! createStrDB / cmdGetDBSub / cmdSetDB in the two late residents.
//! They own the same eight tab-delimited tables and eight column pairs, but
//! their expression evaluators and register destination dispatch are separate.
use crate::{
    Action, cstring, late_numeric, native_numbers,
    registers::{Change, Expression, MidoriStringDb},
    word,
};
use anyhow::{Result, ensure};

// calcExpression, 0x159d40 / 0x159e10. Restrict the shared evaluator to
// late operands verified here. Function calls have their own dispatch.
fn key(raw: &[u8]) -> Result<Expression> {
    if let Ok(value) = late_numeric::scalar(raw) {
        return Ok(value.into());
    }
    let mut depth = 0;
    for token in raw.split(|&b| b == b' ').filter(|t| !t.is_empty()) {
        if matches!(token, b"+" | b"-" | b"*" | b"/") {
            ensure!(depth >= 2, "unverified late string DB expression stack");
            depth -= 1;
        } else {
            ensure!(
                !matches!(token.first(), Some(b'-' | b'+')),
                "unverified late string DB expression sign"
            );
            late_numeric::scalar(token)?;
            depth += 1;
        }
    }
    ensure!(
        depth == 1 && raw.first() != Some(&b'-'),
        "unverified late string DB expression"
    );
    Expression::parse(raw)
}

pub(super) fn decode(bytes: &[u8], start: usize, header: u32) -> Result<(Action, usize)> {
    let op = header as u16;
    let words = usize::from((header >> 16) as u8);
    let string_count = ((header >> 24) & 15) as usize;
    // All three handlers read FCString field zero irrespective of any
    // additional encoded fields. CMD_DATA's string/number arrays are 10/21.
    ensure!(
        words <= 21 && string_count <= 10,
        "unverified late string DB fields"
    );
    let mut end = start + 4 + words * 4;
    let mut raw = &[][..];
    for i in 0..string_count {
        let (value, next) = cstring(bytes, end)?;
        if i == 0 {
            raw = value;
        }
        end = next;
    }
    let cmd = if op == 0xd0 {
        ensure!(words > 0, "missing late string DB table ID");
        MidoriStringDb::create(word(bytes, start + 4)? as i32, raw)?
    } else {
        let mut fields = raw.split(|&b| b == b',');
        let mut field = || fields.next().unwrap_or_default();
        let table = native_numbers::integer2011(field());
        let key = key(field())?;
        let set = op == 0xd2;
        let mut pairs = Vec::with_capacity(8);
        for _ in 0..8 {
            let col = native_numbers::integer2011(field());
            let arg = field();
            if !set && (0..15).contains(&col) {
                // A T target receives raw row bytes. R/S/L consume the
                // column converted through FCString's native integer reader.
                // Other late letRegByString destinations have extra owners.
                ensure!(
                    arg.is_empty()
                        || matches!(
                            arg.first().map(u8::to_ascii_uppercase),
                            Some(b'R' | b'S' | b'L' | b'T' | b'*')
                        ),
                    "unverified late string DB register destination"
                );
            }
            pairs.push((col, arg.to_vec()));
        }
        MidoriStringDb::Access {
            table,
            key,
            pairs,
            set,
        }
    };
    Ok((Action::Change(Change::MidoriStringDb(cmd)), end))
}

#[cfg(test)]
mod tests {
    use crate::{Action, Registers, Script, Value, profile::Resident};
    use anyhow::{Result, bail};

    fn execute(
        profile: Resident,
        regs: &mut Registers,
        op: u16,
        words: &[i32],
        csv: &[u8],
    ) -> Result<()> {
        let mut bytes = vec![0; 20];
        bytes[..4].copy_from_slice(b"\0DLR");
        bytes[4..8].copy_from_slice(&3u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&16u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&1u32.to_le_bytes());
        bytes.extend(op.to_le_bytes());
        // Deliberately include an extra string and high mode bits. Both
        // resident handlers use field zero and the record is fully consumed.
        bytes.extend([words.len() as u8, 0x22]);
        for w in words {
            bytes.extend(w.to_le_bytes());
        }
        bytes.extend(csv);
        bytes.extend(b"\0ignored\0");
        let mut script = Script::new(bytes, 3)?;
        script.resident = Some(profile);
        let Action::Change(c) = script.next()? else {
            bail!("expected late database command")
        };
        regs.change(c)?;
        assert_eq!(script.pc, script.bytes.len());
        Ok(())
    }

    #[test]
    fn late_string_db_updates_are_visible_in_both_residents() -> Result<()> {
        for resident in [Resident::LateVol1, Resident::LateVol2] {
            let mut regs = crate::native_state_tests::fresh(3)?;
            regs.set_local(4, -8)?;
            regs.set_local(9, -7)?;
            execute(
                resident,
                &mut regs,
                0xd0,
                &[2],
                b";comment\r\n-7\told\t1\n-7\t\x82\xa0\t37\tkeep",
            )?;
            execute(resident, &mut regs, 0xd1, &[], b"2,R4 1 +,0,T5,1,R6")?;
            assert_eq!(regs.text(5)?, b"\x82\xa0");
            assert_eq!(regs.read(Value::Local(6))?, 37);
            execute(
                resident,
                &mut regs,
                0xd2,
                &[],
                b"2,R9,0,new,1,42,14,discarded,-1,*,-1,*,-1,*,-1,*,-1,*",
            )?;
            execute(resident, &mut regs, 0xd1, &[], b"2,-7,0,T5,1,R6,2,T7,14,T8")?;
            assert_eq!(regs.text(5)?, b"new");
            assert_eq!(regs.read(Value::Local(6))?, 42);
            assert_eq!(regs.text(7)?, b"keep");
            assert_eq!(regs.text(8)?, b"");
            // Regenerate table 2. Looking up the missing old key must leave
            // the register unchanged; a second database remains independent.
            execute(resident, &mut regs, 0xd0, &[3], b"9\tother")?;
            execute(resident, &mut regs, 0xd0, &[2], b"8\tnewkey")?;
            execute(resident, &mut regs, 0xd1, &[], b"2,-7,0,T5")?;
            assert_eq!(regs.text(5)?, b"new");
            execute(resident, &mut regs, 0xd1, &[], b"3,9,0,T5")?;
            assert_eq!(regs.text(5)?, b"other");
        }
        Ok(())
    }
}
