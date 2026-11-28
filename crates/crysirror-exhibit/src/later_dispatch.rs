// SPDX-License-Identifier: MPL-2.0
//! The late residents fill all 287 method slots before applying 234 writes.
//! liteExec handles thirteen disjoint slots before indexing that table.
use crate::{Action, branch, cstring, midori_state, native_numbers, profile::Resident, registers::Change, word};
use anyhow::{Result, ensure};

pub(super) fn empty_native_method(resident: Resident, op: u16) -> bool {
    // The 2011 default at 0x8f00 and both later defaults return directly.
    // 0x2f is also a distinct label in the original branch-index scan.
    matches!(op, 0 | 2..=4 | 9 | 0x0e..=0x10 | 0x19..=0x1b | 0x1f | 0x4d |
        0x59 | 0x60 | 0x68 | 0x6f | 0x7c | 0x85 | 0x89 | 0x92 | 0x9c |
        0xc9 | 0xca | 0xda | 0xe4 | 0xe5 | 0xe8 | 0xe9 | 0xeb)
        || if resident.late() {
            matches!(op, 0x2f | 0x103..=0x106 | 0x108 | 0x11a..=0x11e)
        } else {
            matches!(op, 1 | 0x100..=0x10a)
        }
}

pub(super) fn check(
    resident: Resident,
    bytes: &[u8],
    start: usize,
    operands_end: usize,
    header: u32,
) -> Result<Option<(Action, usize)>> {
    let op = header as u16;
    if empty_native_method(resident, op) {
        let (_, _, end) = branch::envelope(bytes, start)?;
        return Ok(Some((Action::NativeMethodReturn, end)));
    }
    match op {
        0x06 | 0x08 => return crate::late_numeric::Assignment::decode(bytes, start, operands_end, header).map(Some),
        0x0a => return crate::late_numeric::text_assignment(bytes, start, operands_end, header).map(Some),
        0x07 | 0xf9 | 0xfa => {}
        0xbe | 0xea => {
            // cmdSetSystemSound / cmdSetSystemSound1 in both late residents
            // consume FCString[0]; registering a choice does not start it.
            let (_, first, end) = branch::envelope(bytes, start)?;
            let raw = first.unwrap_or_default();
            let command = if op == 0xbe {
                crate::midori_control_sound::Command::basic(raw)
            } else {
                crate::midori_control_sound::Command::extended(raw)
            };
            return Ok(Some((Action::BindMidoriControlSound(command), end)));
        }
        0xd0..=0xd2 => return crate::later_string_db::decode(bytes, start, header).map(Some),
        0x14 | 0x16 | 0x17 => return crate::later_flow::Flow::decode(bytes, start, operands_end, header).map(Some),
        0x05 if matches!(header >> 16, 0x0102 | 0x1100 | 0x2100) => {}
        0x47 if header >> 16 == 0x2100 => {}
        0x42 if header >> 16 == 0x0100 => {}
        0x40 if header >> 16 & 0x0fff == 0x0100 => {}
        0xfb if header >> 16 == 0x0100 => {}
        0x2d if header >> 16 == 0x0101 => {}
        0x2c if header >> 16 == 0x0100 => {
            let (raw, end) = cstring(bytes, operands_end)?;
            let bundle = crate::midori_control_parts::Notification::bundle(raw)
                .ok_or_else(|| anyhow::anyhow!("unsupported ExHibit {resident:?} control-parts selector"))?;
            return Ok(Some((Action::DefineNotifications(bundle), end)));
        }
        0x100 if header >> 16 == 0x0100 => {
            let (raw, end) = cstring(bytes, operands_end)?;
            let action = crate::midori_control_parts::Notification::parse(raw)
                .map_or(Action::NativeMethodReturn, Action::DefineNotification);
            return Ok(Some((action, end)));
        }
        0x3d..=0x3f => {
            // Both late residents keep eight INTDB tables and signed keys.
            // Their expression evaluators are separate from the 2011 one.
            let (raw, end) = cstring(bytes, operands_end)?;
            let scalar = |field: &[u8]| crate::late_numeric::scalar(field).map(Into::into);
            let change = match (op, header >> 16) {
                (0x3d, 0x0101) => Change::MidoriDefineDb(midori_state::DbDefinition::parse(
                    word(bytes, start + 4)? as i32, raw,
                )?),
                (0x3e, 0x1100) => Change::MidoriGetDb(midori_state::DbLookup::parse_with(raw, scalar)?),
                (0x3f, 0x1100) => Change::MidoriSetDb(midori_state::DbWrite::parse_with(raw, scalar)?),
                _ => anyhow::bail!("unsupported ExHibit {resident:?} integer table encoding {header:#010x}"),
            };
            return Ok(Some((Action::Change(change), end)));
        }
        0x0b | 0xbf => {
            ensure!(
                header >> 16 == 0x100,
                "unsupported ExHibit {resident:?} opcode {op:#06x} argument encoding"
            );
            let (csv, end) = cstring(bytes, operands_end)?;
            let mut fields = csv.splitn(7, |b| *b == b',');
            let id = native_numbers::integer2011(fields.next().unwrap_or_default());
            if (op, id) == (0xbf, 271) {
                return Ok(Some((Action::LateScreenUpdate(crate::later_screen::Command::parse(fields)?), end)));
            }
            ensure!(
                matches!((op, id), (0x0b, 30) | (0xbf, 342)),
                "unsupported ExHibit {resident:?} opcode {op:#06x} selector {id}"
            );
            if op == 0xbf {
                // These residents have their own calcExpression. Admit the
                // scalar integer inputs established by the original calls.
                for field in fields.skip(1).take(4) {
                    ensure!(
                        field.is_empty()
                            || std::str::from_utf8(field)
                                .ok()
                                .and_then(|s| s.parse::<i32>().ok())
                                .is_some(),
                        "unverified ExHibit {resident:?} action {id} expression"
                    );
                }
            }
        }
        _ => anyhow::bail!("unsupported ExHibit {resident:?} execution opcode {op:#06x}"),
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Script, Value};

    #[test]
    fn late_sound_bindings_replace_and_clear_the_native_control_slots() -> Result<()> {
        for resident in [Resident::LateVol1, Resident::LateVol2] {
            let mut records = Vec::new();
            let mut push = |op: u16, data: &[u8]| {
                records.extend(op.to_le_bytes());
                records.extend([1, 0x22]);
                records.extend(97i32.to_le_bytes());
                records.extend(data);
                records.extend(b"\0unused\0");
            };
            push(0xbe, b"-1,10.20.30");
            push(0xea, b"2,42,1,custom.wav,first,57,extra;42,duplicate;57,duplicate");
            push(0xbe, b"100,250");
            push(0xea, b"99,7,2,last.wav,label");
            let mut script = Script::new(crate::scenario_load_tests::scenario(3, &records, 4), 3)?;
            script.resident = Some(resident);
            let mut catalog = crate::midori_control_sound::Catalog::default();
            let mut apply = |catalog: &mut crate::midori_control_sound::Catalog| -> Result<()> {
                let Action::BindMidoriControlSound(command) = script.next()? else {
                    anyhow::bail!("expected registered control sound");
                };
                catalog.apply(command);
                Ok(())
            };
            apply(&mut catalog)?;
            assert_eq!([catalog.ids[&0], catalog.ids[&1], catalog.ids[&2]], [10, 20, 30]);
            apply(&mut catalog)?;
            let details = serde_json::to_value(&catalog.details[&2])?;
            assert_eq!(details["filename"], serde_json::json!(b"custom.wav"));
            assert_eq!(details["choices"], serde_json::json!([
                {"id":42,"label":[0x95,0x57,0x8f,0x80,0x89,0xb9]},
                {"id":57,"label":b"extra"},
            ]));
            apply(&mut catalog)?;
            assert_eq!(catalog.ids.len(), 3);
            apply(&mut catalog)?;
            assert_eq!(catalog.ids.len(), 100);
            assert_eq!(catalog.ids[&99], 7);
            assert!(catalog.ids.iter().all(|(k, id)| *k == 99 || *id == -1));
            // Reset preserves filenames on unaffected slots and empties choices.
            let old = serde_json::to_value(&catalog.details[&2])?;
            assert_eq!(old["filename"], details["filename"]);
            assert_eq!(old["choices"], serde_json::json!([]));
            assert_eq!(script.remaining, 0);
        }
        Ok(())
    }

    #[test]
    fn midori_default_methods_keep_the_following_record_and_late_ids_distinct() -> Result<()> {
        let midori = Resident::Midori2011;
        assert_eq!((0..=0x10a).filter(|op| empty_native_method(midori, *op)).count(), 42);
        assert!(!empty_native_method(midori, 0x2f));
        assert!(!empty_native_method(Resident::LateVol1, 0x100));
        let records = b"\0\x01\x01\x01\x7f\0\0\0ignored\0\x0d\0\x01\0\x19\0\0\0";
        let mut script = Script::new(crate::scenario_load_tests::scenario(3, records, 2), 3)?;
        script.resident = Some(midori);
        assert_eq!(script.next()?, Action::NativeMethodReturn);
        assert_eq!(script.next()?, Action::Wait(25));
        assert_eq!(script.remaining, 0);
        Ok(())
    }

    #[test]
    fn v3_precache_dispatches_the_same_scoped_read_with_the_native_limit() -> Result<()> {
        let records = b"\x2d\0\x01\x01\x09\0\0\0.\\res\\clip.dat\0";
        for resident in [Resident::Midori2011, Resident::LateVol1, Resident::LateVol2] {
            let mut script = Script::new(crate::scenario_load_tests::scenario(3, records, 1), 3)?;
            script.resident = Some(resident);
            let Action::PreCache(Some(path), limit) = script.next()? else {
                anyhow::bail!("missing v3 precache action");
            };
            assert_eq!(path.relative(), "res/clip.dat");
            assert_eq!(limit, 9);
            assert_eq!(script.remaining, 0);
        }
        Ok(())
    }

    #[test]
    fn late_integer_tables_keep_signed_rows_and_use_live_register_operands() -> Result<()> {
        for resident in [Resident::LateVol1, Resident::LateVol2] {
            let mut records = Vec::new();
            let mut push = |header: u32, words: &[i32], raw: &[u8]| {
                records.extend(header.to_le_bytes());
                for value in words {
                    records.extend(value.to_le_bytes());
                }
                records.extend(raw);
                records.push(0);
            };
            push(0x0101_003d, &[3], b"; ignored\r\n-5 1 2\n-5,0x10,0b11\n7,99");
            push(0x1100_003e, &[], b"3,R10,0,2,1,3,8,4");
            push(0x1100_003f, &[], b"3,R10,1,R11,-1,0,0,999");
            push(0x1100_003e, &[], b"3,-5,0,5,1,6");
            push(0x1100_003e, &[], b"3,6,0,5,1,6");
            push(0x0101_003d, &[3], b"7,20");
            push(0x1100_003e, &[], b"3,-5,0,5");
            let mut script = Script::new(crate::scenario_load_tests::scenario(3, &records, 7), 3)?;
            script.resident = Some(resident);
            let mut regs = crate::native_state_tests::fresh(3)?;
            regs.set_local(10, -5)?;
            regs.set_local(11, -123)?;
            for _ in 0..7 {
                let Action::Change(change) = script.next()? else {
                    anyhow::bail!("expected a native integer-table operation");
                };
                regs.change(change)?;
            }
            assert_eq!(regs.read(Value::Local(2))?, 16);
            assert_eq!(regs.read(Value::Local(3))?, 3);
            assert_eq!(regs.read(Value::Local(4))?, 0);
            assert_eq!(regs.read(Value::Local(5))?, 16);
            assert_eq!(regs.read(Value::Local(6))?, -123);
        }
        Ok(())
    }

    #[test]
    fn late_profiles_use_native_empty_methods_and_calc_reg_without_midori_fallback() -> Result<()> {
        assert_eq!((0..=0x11e).filter(|op| empty_native_method(Resident::LateVol1, *op)).count(), 41);
        for profile in [Resident::LateVol1, Resident::LateVol2] {
            let mut bytes = vec![0; 20];
            bytes[..4].copy_from_slice(b"\0DLR");
            bytes[4..8].copy_from_slice(&3u32.to_le_bytes());
            bytes[8..12].copy_from_slice(&16u32.to_le_bytes());
            bytes[12..16].copy_from_slice(&3u32.to_le_bytes());
            bytes.extend(0x0101_00dau32.to_le_bytes());
            bytes.extend(33u32.to_le_bytes());
            bytes.extend(b"unused\0");
            bytes.extend(0x0007_0007u32.to_le_bytes());
            for operand in [1i32, 27, 0, 0, 15, 0, 3] {
                bytes.extend(operand.to_le_bytes());
            }
            bytes.extend(0x0100_00bfu32.to_le_bytes());
            bytes.extend(b"271,-1,1,0,0,0,*\0");
            let mut script = Script::new(bytes, 3)?;
            script.resident = Some(profile);
            assert_eq!(script.next()?, Action::NativeMethodReturn);
            let Action::Change(change) = script.next()? else { anyhow::bail!("expected calcReg") };
            let mut regs = crate::native_state_tests::fresh(3)?;
            regs.change(change)?;
            assert_eq!(regs.read(Value::Local(27))?, 18);
            assert!(matches!(script.next()?, Action::LateScreenUpdate(_)));
            assert_eq!(script.remaining, 0);
        }
        Ok(())
    }
}
