// SPDX-License-Identifier: MPL-2.0
//! Detached Midori block-entry pieces driven by present VM owners.
use crate::{
    midori_message::{Catalog, SocketCapture},
    midori_state::EntryNames,
    registers::{ExperienceCapture, Registers},
};
use anyhow::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MaskedNormal {
    pub register: u32,
    pub mask: i32,
    pub bits: i32,
}

impl MaskedNormal {
    pub fn authored_block_flag(enabled: bool) -> Self {
        // blockEntry copies bit 2 of the native view flags into R4 bit 7.
        // The already-scoped block parser owns this flag independently.
        Self {
            register: 4,
            mask: 0x80,
            bits: if enabled { 0x80 } else { 0 },
        }
    }

    pub fn printer(open: bool) -> Self {
        // 0x108d20 also captures local controls and engine state. This
        // producer neither clears nor supplies those other R4 fields.
        Self {
            register: 4,
            mask: 1,
            bits: i32::from(open),
        }
    }
}

pub(super) struct StatusCapture {
    pub printer_open: Option<MaskedNormal>,
    pub printer_socket: Option<SocketCapture>,
    pub entry_names: (u32, Vec<u8>),
    pub experience: ExperienceCapture,
}

impl StatusCapture {
    pub fn from_owners(regs: &Registers, printer: &Catalog, names: &EntryNames) -> Result<Self> {
        Ok(Self {
            printer_open: printer.checkpoint_open().map(MaskedNormal::printer),
            printer_socket: printer.checkpoint_socket(),
            entry_names: (207, names.checkpoint_text()),
            experience: regs.capture_experience()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        midori_state::{EntryAssignment, SceneDone},
        registers::{Bank, Change, Limits, Value},
    };

    fn profile_registers() -> Result<Registers> {
        Registers::for_scenario(
            Some(Limits {
                regs: 2048,
                sys: 4864,
                strings: 256,
                cg: 2500,
                scene: 400,
                sound: 100,
                characters: 34,
                messages: 48000,
                locals: 100,
            }),
            3,
            [1024, 576],
        )
    }

    #[test]
    fn midori_checkpoint_experience_reads_actual_banks_and_masks_only_the_printer_bit() -> Result<()>
    {
        let mut regs = profile_registers()?;
        let printer = Catalog::default();
        let mut names = EntryNames::default();
        assert_eq!(regs.read(Value::System(16))?, 217);
        regs.set_normal(4, 0x9e)?;
        regs.set_system(216, 37)?;
        regs.set_system(217, -17)?;
        regs.set_system(4720, -46)?;
        regs.set_system(4721, 99)?;
        names.set(EntryAssignment::parse(b"30,-1,2,4,synthetic")?);
        names.set(EntryAssignment::parse(b"30,-1,2,7,ending")?);
        regs.change(Change::Experience(Bank::Cg, vec![0, 2499, 2499, 2500]))?;
        regs.change(Change::MidoriSceneDone(SceneDone(12)))?;
        regs.change(Change::Experience(Bank::Sound, vec![11, 99, 100]))?;
        for id in [0, 7, 24, 31, 32, 47999, 48000] {
            regs.mark_message(id)?;
        }

        let status = StatusCapture::from_owners(&regs, &printer, &names)?;
        assert!(status.printer_socket.is_none());
        let open = status.printer_open.unwrap();
        assert_eq!(
            open,
            MaskedNormal {
                register: 4,
                mask: 1,
                bits: 0
            }
        );
        assert_eq!(
            status.entry_names,
            (207, b"8,0.,1.,2.,3.,4.synthetic,5.,6.,7.ending".to_vec())
        );
        let capture = status.experience;
        assert_eq!(
            (capture.base, capture.words, capture.counts),
            (217, 4504, [2500, 400, 100, 1500])
        );
        assert_eq!(
            (0..4).map(|i| capture.cell(i).unwrap()).collect::<Vec<_>>(),
            [2500, 400, 100, 1500]
        );
        assert_eq!(capture.cell(4), Some(1));
        assert_eq!(capture.cell(5), Some(0)); // explicit zero in the captured span
        assert_eq!(capture.cell(2503), Some(2)); // last CG
        assert_eq!(capture.cell(2504 + 12), Some(1));
        assert_eq!(capture.cell(2904 + 11), Some(1));
        assert_eq!(capture.cell(3003), Some(1)); // last sound
        // mark_message stores MSB-first within each byte; retaining a
        // high bit in the first DWORD must preserve its signed bit pattern.
        assert_eq!(
            capture.cell(3004),
            Some(0x8100_0081u32.wrapping_add(1 << 11) as i32)
        );
        assert_eq!(capture.cell(3005), Some(128));
        assert_eq!(capture.cell(4503), Some(1 << 24));
        assert_eq!(capture.cell(4504), None);
        assert_eq!(capture.nonzero_words(), 12);

        assert_eq!(regs.read(Value::Local(4))?, 0x9e);
        assert_eq!(regs.read(Value::System(216))?, 37);
        assert_eq!(regs.read(Value::System(217))?, -17);
        assert_eq!(regs.read(Value::System(4720))?, -46);
        assert_eq!(regs.read(Value::System(4721))?, 99);
        assert!(regs.text(207)?.is_empty());

        regs.set_system(16, 360)?;
        let moved = StatusCapture::from_owners(&regs, &printer, &names)?.experience;
        assert_eq!(moved.base + moved.words as i32, 4864); // exact end is valid
        assert_eq!(moved.cell(2504 + 12), Some(1));
        regs.set_system(16, 361)?;
        assert!(StatusCapture::from_owners(&regs, &printer, &names).is_err());
        regs.set_system(16, -1)?;
        assert!(StatusCapture::from_owners(&regs, &printer, &names).is_err());
        Ok(())
    }
}
