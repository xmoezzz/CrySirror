// SPDX-License-Identifier: MPL-2.0
//! The 2011 resident's state-only cmdAction methods. The call envelope
//! evaluates all four arguments before any method writes to the holder.
use super::{Limits, Registers, Value};
use anyhow::{Context, Result, ensure};

#[derive(Debug, PartialEq)]
pub(crate) enum Setting {
    SkipMode(bool),
    ReadSystem(i32),
    ReadBits { slot: i32, shift: u32, mask: u32 },
    ReadLocalFlag(i32),
    VoiceLevel,
    VoiceMuted,
    ReadSystemFlag(i32, bool),
    IsDefaultSkip,
    Fraction,
    Interpolate,
    WriteGlobal,
    WriteSystem,
    WriteText,
    WriteRelative,
    WriteRelativeFive,
    WriteFixed(i32),
    SetWait(i32),
    FlagEqualsOne,
    FlagNonzero,
    SetNibble24,
    SetMovieLevel,
    SetMuteBit(u32),
    SetVoiceMute,
    PreserveLocalFlag,
    SetControlVolume,
}

impl Setting {
    pub(super) fn for_selector(code: i32) -> Option<Self> {
        Some(match code {
            167 | 168 => Self::SkipMode(code == 168),
            200..=205 => Self::ReadSystem(code - 183),
            206 => Self::ReadSystem(13),
            207 => Self::ReadSystem(12),
            208..=210 => Self::ReadBits {
                slot: code - 177,
                shift: 16,
                mask: 0xffff,
            },
            211 => Self::VoiceLevel,
            215 => Self::VoiceMuted,
            212..=214 => Self::ReadBits {
                slot: 30,
                shift: [16, 18, 17][(code - 212) as usize],
                mask: 1,
            },
            216 => Self::ReadSystemFlag(0, false),
            217 => Self::ReadBits {
                slot: 24,
                shift: 12,
                mask: 15,
            },
            218 => Self::ReadBits {
                slot: 24,
                shift: 0,
                mask: 0xfff,
            },
            220 => Self::ReadLocalFlag(0),
            221..=223 => Self::ReadLocalFlag(code - 219),
            224 => Self::ReadSystemFlag(1, true),
            226 => Self::IsDefaultSkip,
            298 => Self::Fraction,
            299 => Self::Interpolate,
            300 => Self::WriteGlobal,
            301 => Self::WriteSystem,
            302 => Self::WriteText,
            303 => Self::WriteRelative,
            304 => Self::WriteRelativeFive,
            306..=311 => Self::WriteFixed(code - 289),
            312 => Self::SetWait(13),
            313 => Self::SetWait(12),
            324..=326 => Self::SetMuteBit([16, 18, 17][(code - 324) as usize]),
            327 => Self::SetVoiceMute,
            339 => Self::PreserveLocalFlag,
            330 => Self::SetMovieLevel,
            336 => Self::SetControlVolume,
            328 => Self::FlagEqualsOne,
            329 => Self::SetNibble24,
            335 => Self::FlagNonzero,
            _ => return None,
        })
    }

    pub(super) fn apply(
        self,
        regs: &mut Registers,
        limits: Limits,
        args: [i32; 4],
        text: Vec<u8>,
    ) -> Result<i32> {
        let [a, b, c, d] = args;
        match self {
            Self::ReadSystem(slot) => return regs.read(Value::System(slot)),
            Self::ReadBits { slot, shift, mask } => {
                return Ok(((regs.read(Value::System(slot))? as u32 >> shift) & mask) as i32);
            }
            Self::VoiceLevel | Self::VoiceMuted => {
                let controls = regs.read(Value::System(30))? as u32;
                let volume = self == Self::VoiceLevel;
                if controls & 1 == 0 {
                    return Ok(if volume {
                        (regs.read(Value::System(34))? as u32 >> 16) as i32
                    } else {
                        ((controls >> 19) & 1) as i32
                    });
                }
                if a < 0 || a as usize >= limits.characters + 3 {
                    return Ok(if volume { 256 } else { 0 });
                }
                let value = regs.read(Value::System(43 + 2 * a + i32::from(volume)))?;
                return Ok(if volume { value } else { i32::from(value != 0) });
            }
            Self::ReadLocalFlag(bit) => return regs.read(Value::Flag(bit)),
            Self::ReadSystemFlag(bit, invert) => {
                return Ok(regs.read(Value::SystemFlag(bit))? ^ i32::from(invert));
            }
            Self::IsDefaultSkip => return Ok(i32::from(regs.read(Value::System(10))? & 0xf0 == 0)),
            Self::Fraction => {
                // RVA 0xb99d0: integer MUL wraps before the signed divide.
                if d == c {
                    return Ok(0);
                }
                return a
                    .wrapping_sub(c)
                    .wrapping_mul(b)
                    .checked_div(d.wrapping_sub(c))
                    .context("ExHibit action 298 signed divide overflow");
            }
            Self::Interpolate => {
                if b == 0 || a == b {
                    return Ok(d);
                }
                let product = a.wrapping_mul(d.wrapping_sub(c));
                let quotient = product
                    .checked_div(b)
                    .context("ExHibit action 299 signed divide overflow")?;
                return Ok(c
                    .wrapping_add(quotient)
                    .wrapping_add(i32::from(product % b != 0)));
            }
            Self::SkipMode(alternate) => {
                let flags = regs.read(Value::System(10))?;
                regs.set_system(10, (flags & !0xf0) | (i32::from(alternate) << 4))?;
            }
            Self::WriteGlobal => regs.set_normal(a, b)?,
            Self::WriteSystem => regs.set_system(a, b)?,
            Self::WriteText => {
                ensure!(
                    (a as usize) < limits.strings,
                    "native action 302 text index {a} out of bounds"
                );
                regs.strings.insert(a, text);
            }
            Self::WriteRelative | Self::WriteRelativeFive => {
                let (offset, value) = if self == Self::WriteRelative {
                    (a, b)
                } else {
                    (5, a)
                };
                let slot = regs.read(Value::System(15))?.wrapping_add(offset);
                regs.set_system(slot, value)?;
            }
            Self::WriteFixed(slot) => regs.set_system(slot, a)?,
            Self::SetWait(slot) => {
                if a >= 0 {
                    regs.set_system(slot, a)?;
                }
            }
            Self::FlagEqualsOne | Self::FlagNonzero => {
                let (mask, enabled) = if self == Self::FlagEqualsOne {
                    (1, a == 1)
                } else {
                    (4, a != 0)
                };
                let flags = regs.read(Value::System(23))?;
                regs.set_system(23, if enabled { flags | mask } else { flags & !mask })?;
            }
            Self::SetMuteBit(bit) => {
                let flags = regs.read(Value::System(30))?;
                let mask = 1 << bit;
                regs.set_system(30, if a == 0 { flags & !mask } else { flags | mask })?;
            }
            Self::SetVoiceMute => {
                let controls = regs.read(Value::System(30))?;
                let enabled = a != 0;
                if controls & 1 == 0 {
                    let mask = 1 << 19;
                    regs.set_system(
                        30,
                        if enabled {
                            controls | mask
                        } else {
                            controls & !mask
                        },
                    )?;
                } else {
                    let additional = text
                        .split(|&ch| ch == b'.')
                        .filter(|part| !part.is_empty())
                        .map(crate::native_numbers::integer2011);
                    for id in std::iter::once(b).chain(additional) {
                        if id >= 0 && (id as usize) < limits.characters + 3 {
                            let slot = 43 + id * 2;
                            if (regs.read(Value::System(slot))? != 0) != enabled {
                                regs.set_system(slot, i32::from(enabled))?;
                            }
                        }
                    }
                }
            }
            Self::PreserveLocalFlag => {
                // action339 uses one saved value initialized to -1 (RVA 0xc59ee).
                let old = regs.read(Value::Flag(2))? != 0;
                let desired = match a {
                    0 => {
                        regs.saved_flag2 = Some(old);
                        match b {
                            0 => Some(true),
                            1 => Some(false),
                            _ => None,
                        }
                    }
                    1 => regs.saved_flag2.take(),
                    _ => None,
                };
                if let Some(enabled) = desired.filter(|&value| value != old) {
                    regs.set_local_flag2011(2, enabled, limits)?;
                }
            }
            Self::SetMovieLevel => {
                let flags = regs.read(Value::System(24))?;
                regs.set_system(24, (flags & !0xfff) | (a & 0xfff))?;
            }
            Self::SetControlVolume => {
                let gain = a.clamp(0, 256);
                regs.set_system(if d == 1 { 36 } else { 35 }, (gain << 16) | gain)?;
            }
            Self::SetNibble24 => {
                let old = regs.read(Value::System(24))?;
                regs.set_system(24, (old & !0xf000) | ((a & 15) << 12))?;
            }
        }
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Script};

    fn regs() -> Result<Registers> {
        Registers::for_scenario(
            Some(Limits {
                regs: 256,
                sys: 256,
                cg: 1,
                scene: 1,
                sound: 1,
                strings: 256,
                characters: 2,
                messages: 80,
                locals: 20,
            }),
            3,
            [1024, 576],
        )
    }

    fn run(regs: &mut Registers, records: &[&[u8]]) -> Result<()> {
        let mut bytes = b"\0DLR".to_vec();
        bytes.extend(3_u32.to_le_bytes());
        bytes.extend(12_u32.to_le_bytes());
        bytes.extend((records.len() as u32).to_le_bytes());
        for record in records {
            bytes.extend(0x0100_00bf_u32.to_le_bytes());
            bytes.extend(*record);
            bytes.push(0);
        }
        let mut script = Script::new(bytes, 3)?;
        while script.remaining > 0 {
            let Action::Change(change) = script.next()? else {
                anyhow::bail!("not a state call")
            };
            regs.change(change)?;
        }
        assert_eq!(script.pc, script.bytes.len());
        Ok(())
    }

    #[test]
    fn setters_queries_and_local_result_use_the_live_banks() -> Result<()> {
        let mut regs = regs()?;
        run(
            &mut regs,
            &[
                b"300,-1,5,73",
                b"306,-1,R5",
                b"200,5",
                b"300,-1,5,99",
                b"312,-1,1444",
                b"312,-1,-5",
                b"206,6",
                b"302,-1,9,0,0,0,\x82\xa0,tail",
                b"303,-1,3,55",
                b"304,-1,64",
            ],
        )?;
        assert_eq!(regs.read(Value::Local(5))?, 73);
        assert_eq!(regs.read(Value::Local(6))?, 1444);
        assert_eq!(regs.normal.get(&5), Some(&99));
        assert_eq!(regs.read(Value::RelativeSystem(1_000_003))?, 55);
        assert_eq!(regs.read(Value::RelativeSystem(1_000_005))?, 64);
        assert_eq!(regs.text(9)?, b"\x82\xa0,tail");
        run(&mut regs, &[b"302,7,9,0,0,0,*"])?;
        assert_eq!(regs.text(9)?, b"");
        assert_eq!(regs.read(Value::Local(7))?, 0);
        Ok(())
    }

    #[test]
    fn packed_settings_keep_adjacent_bits_and_query_unsigned_halves() -> Result<()> {
        let mut regs = regs()?;
        run(
            &mut regs,
            &[
                b"301,-1,31,0xff001234",
                b"208,1",
                b"301,-1,24,0xf00d1025",
                b"329,-1,30",
                b"217,2",
                b"218,3",
                b"168,-1",
                b"226,4",
                b"167,-1",
                b"226,5",
                b"328,-1,-1",
                b"216,6",
                b"335,-1,-1",
                b"335,-1,0",
            ],
        )?;
        assert_eq!(regs.read(Value::Local(1))?, 0xff00);
        assert_eq!(regs.read(Value::Local(2))?, 14);
        assert_eq!(regs.read(Value::Local(3))?, 0x25);
        assert_eq!(regs.read(Value::Local(4))?, 0);
        assert_eq!(regs.read(Value::Local(5))?, 1);
        assert_eq!(regs.read(Value::Local(6))?, 0);
        assert_eq!(regs.read(Value::System(24))? as u32, 0xf00de025);
        assert_eq!(regs.read(Value::SystemFlag(2))?, 0);
        Ok(())
    }

    #[test]
    fn script_ignores_view_only_actions_after_evaluating_arguments() -> Result<()> {
        use super::super::assigned_action_2011;
        assert_eq!((0..400).filter(|&n| assigned_action_2011(n)).count(), 232);
        let mut regs = regs()?;
        run(
            &mut regs,
            &[b"301,-1,24,0x12400", b"300,-1,2,55", b"52,2,3"],
        )?;
        assert_eq!(regs.read(Value::Local(2))?, 0);
        assert_eq!(regs.normal[&2], 55);
        assert!(run(&mut regs, &[b"52,2,Q2"]).is_err());
        assert!(run(&mut regs, &[b"99,2,0"]).is_err());
        run(
            &mut regs,
            &[
                b"300,-1,3,89",
                b"80,3",
                b"1000001,4",
                b"154,-1",
                b"171,-1",
                b"220,5",
                b"172,-1",
                b"220,6",
                b"152,-1",
                b"199,-1",
                b"324,-1,-1",
                b"212,7",
                b"324,-1,0",
                b"212,8",
                b"330,-1,0x1002",
                b"336,-1,500,0,0,1",
                b"336,-1,-1",
            ],
        )?;
        assert_eq!(regs.read(Value::Local(3))?, 0);
        assert_eq!(regs.read(Value::Local(4))?, 0);
        assert_eq!(regs.read(Value::Local(5))?, 1);
        assert_eq!(regs.read(Value::Local(6))?, 0);
        assert_eq!(regs.read(Value::Local(7))?, 1);
        assert_eq!(regs.read(Value::Local(8))?, 0);
        assert_eq!(regs.read(Value::SystemFlag(2))?, 1);
        assert_eq!(regs.read(Value::Flag(6))?, 0);
        assert_eq!(regs.read(Value::System(24))?, 0x12002);
        assert_eq!(regs.read(Value::System(36))?, 0x01000100);
        assert_eq!(regs.read(Value::System(35))?, 0);
        Ok(())
    }

    #[test]
    fn mute_lists_and_saved_flag_use_independent_native_state() -> Result<()> {
        let mut regs = regs()?;
        run(
            &mut regs,
            &[
                b"301,-1,30,1",
                b"301,-1,47,9",
                b"327,-1,1,1,0,0,2..4.-1.5",
                b"215,1,1",
                b"215,2,2",
                b"215,3,4",
                b"327,-1,0,2,0,0,1.4",
                b"215,4,4",
                b"301,-1,30,0x320",
                b"327,-1,1,-9",
                b"215,5,-1",
                b"327,-1,0,70",
                b"215,6,-1",
                b"174,-1",
                b"339,-1,0,0",
                b"221,7",
                b"339,-1,1",
                b"221,8",
                b"173,-1",
                b"339,-1,1",
                b"221,9",
            ],
        )?;
        for (slot, value) in [
            (1, 1),
            (2, 1),
            (3, 1),
            (4, 0),
            (5, 1),
            (6, 0),
            (7, 1),
            (8, 0),
            (9, 1),
        ] {
            assert_eq!(regs.read(Value::Local(slot))?, value);
        }
        assert_eq!(regs.read(Value::System(30))?, 0x320);
        Ok(())
    }

    #[test]
    fn native_voice_queries_follow_global_and_character_settings() -> Result<()> {
        let mut regs = regs()?;
        run(
            &mut regs,
            &[
                b"301,-1,30,0x80000",
                b"301,-1,34,0xffaa0055",
                b"211,1,-1",
                b"215,2,42",
                b"301,-1,30,1",
                b"301,-1,51,-1",
                b"301,-1,52,37",
                b"211,3,4",
                b"215,4,4",
                b"211,5,5",
                b"215,6,-1",
            ],
        )?;
        for (slot, value) in [(1, 0xffaa), (2, 1), (3, 37), (4, 1), (5, 256), (6, 0)] {
            assert_eq!(regs.read(Value::Local(slot))?, value);
        }
        Ok(())
    }

    #[test]
    fn action_programs_use_native_stack_order_live_state_and_function_operands() -> Result<()> {
        let mut regs = regs()?;
        regs.mark_message(9)?;
        run(
            &mut regs,
            &[
                b"300,-1,5,9",
                b"306,-1,R5 7 + 2 *",
                b"200,0",
                b"306,-1,BR5.3 MVR5 +",
                b"200,1",
                b"306,-1,7 +",
                b"200,2",
                b"306,-1,9 0 / 2 +",
                b"200,3",
                b"306,-1,@mod(2'19'R5)",
                b"200,4",
                b"306,-1,@distance(4'0'0'3'4)",
                b"200,5",
                b"306,-1,@dir8(4'2'2'1'1)",
                b"200,6",
                b"306,-1,@isNear(5'5'0'0'3'4)",
                b"200,7",
                b"306,-1,@clip(4'11'2'10'1)",
                b"200,8",
                b"306,-1,@isString(1'20)",
                b"200,9",
                b"302,-1,20,0,0,0,hello",
                b"306,-1,@isNotEmpty(1'20)",
                b"200,10",
                b"341,-1,3,1",
                b"306,-1,@isCursor(1'1)",
                b"200,11",
            ],
        )?;
        for (slot, value) in [32, 2, 14, 0, 1, 25, 8, 1, 2, 1, 1, 1]
            .into_iter()
            .enumerate()
        {
            assert_eq!(regs.read(Value::Local(slot as i32))?, value);
        }
        // A failure in the unused fourth parameter still precedes all writes.
        assert!(run(&mut regs, &[b"306,11,44,0,0,@clip(2'3'6)"]).is_err());
        assert!(run(&mut regs, &[b"306,11,44,0,0,Q2"]).is_err());
        assert_eq!(regs.read(Value::System(17))?, 1);
        assert_eq!(regs.read(Value::Local(11))?, 1);
        Ok(())
    }

    #[test]
    fn conditional_branch_uses_the_same_installed_expression_operators() -> Result<()> {
        use crate::branch::Condition;
        let mut regs = regs()?;
        let conditional =
            Condition::for_abi(b"@isNotEmpty(1'12)&R5 4 *\x81\x8620;BR5.0\x81\x810", 3)?;
        regs.change(crate::registers::assignment_for_abi(
            6,
            b"1,5,0,@dist(4'0'0'1'2),*,",
            3,
        )?)?;
        regs.change(crate::registers::assignment_for_abi(
            8,
            b"1,100,0,R5 2 *,*,",
            3,
        )?)?;
        assert_eq!(regs.read(Value::System(100))?, 10);
        assert!(!conditional.matches(&regs)?);
        run(&mut regs, &[b"302,-1,12,0,0,0,hello"])?;
        assert!(conditional.matches(&regs)?);
        regs.set_local(5, 3)?;
        assert!(!conditional.matches(&regs)?);
        regs.set_local(5, 2)?;
        assert!(conditional.matches(&regs)?);
        assert!(Condition::for_abi(b"T12\x81\x81T13", 3).is_err());
        Ok(())
    }

    #[test]
    fn native_range_conversions_wrap_before_signed_division() -> Result<()> {
        let mut regs = regs()?;
        run(
            &mut regs,
            &[
                b"298,0,42,5,11,11",
                b"298,1,-5,100,5,25",
                b"299,2,1,3,5,3",
                b"299,3,0,0,1,29",
                b"299,4,3,3,5,41",
                b"298,5,2147483647,2,0,2",
            ],
        )?;
        let read = |n| regs.read(Value::Local(n));
        assert_eq!(
            [read(0)?, read(1)?, read(2)?, read(3)?, read(4)?, read(5)?],
            [0, -50, 6, 29, 41, -1]
        );
        Ok(())
    }
}
