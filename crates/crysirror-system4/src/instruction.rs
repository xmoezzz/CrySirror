// Copyright (c) 2026 CrySirror contributors. GPL-2.0-or-later.
//! Decode original v5 instruction operands without executing or acknowledging
//! native operations. A recognized layout does not establish VM support.
use crate::{ain::Program, opcodes};
use anyhow::{Context, Result, ensure};
#[derive(Clone, Debug)]
pub struct Instruction {
    pub pc: u32,
    pub next: u32,
    pub opcode: u16,
    pub name: &'static str,
    pub arguments: Vec<i32>,
}
impl Program {
    pub fn instruction(&self, pc: u32) -> Result<Instruction> {
        ensure!(self.version == 5, "AIN instruction version");
        let at = pc as usize;
        let end = at
            .checked_add(2)
            .context("AIN instruction address overflow")?;
        let raw = self.code.get(at..end).context("AIN truncated opcode")?;
        let opcode = u16::from_le_bytes(raw.try_into().unwrap());
        let (name, argc) = opcodes::layout(opcode)
            .with_context(|| format!("unknown AIN layout opcode={opcode:#x} pc={pc:#x}"))?;
        let next = end
            .checked_add(argc * 4)
            .context("AIN instruction length overflow")?;
        let raw = self.code.get(end..next).context("AIN truncated operands")?;
        let arguments = raw
            .chunks_exact(4)
            .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        Ok(Instruction {
            pc,
            next: u32::try_from(next)?,
            opcode,
            name,
            arguments,
        })
    }
    /// Validate a complete code stream and function entry boundaries. Does not
    /// execute unknown native calls or claim those instructions are implemented.
    pub fn validate_code_layout(&self) -> Result<usize> {
        let mut pc = 0;
        ensure!(
            self.code.len() <= crate::ain::MAX_AIN_BYTES,
            "AIN code budget"
        );
        // One bit per code byte caps validation storage at 8 MiB, including
        // hostile all-two-byte instruction streams at the maximum input size.
        let mut boundaries = vec![0u8; self.code.len().div_ceil(8)];
        let mut count = 0;
        while (pc as usize) < self.code.len() {
            let index = pc as usize;
            boundaries[index / 8] |= 1 << (index % 8);
            pc = self.instruction(pc)?.next;
            count += 1;
        }
        for function in &self.functions {
            ensure!(
                boundaries
                    .get(function.address as usize / 8)
                    .is_some_and(|b| b & (1 << (function.address % 8)) != 0),
                "AIN function entry inside operand bytes"
            );
        }
        Ok(count)
    }
}
