// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Bounded native structured-branch addresses, not host/UI navigation.
use crate::{
    expression::{evaluate, Variables},
    Program,
};
use anyhow::{ensure, Context, Result};
pub(crate) fn case_target(program: &Program, mut pc: u32, selected: u16) -> Result<u32> {
    let first = program.instruction(pc)?;
    if first.identify == CONTROL && first.code == OPEN {
        pc = first.next;
    }
    for _ in 0..10000 {
        let case = program.instruction(pc)?;
        // The original returns the first non-CASE as the fallback body.
        if case.identify != CONTROL || case.code != 0x55560008 {
            return target(program, pc);
        }
        let id = *case.operands.first().context("SPT CASE value absent")? as u16;
        if id == selected {
            return target(program, case.next);
        }
        pc = skip_body(program, case.next)?;
        if pc as usize == program.word_count() {
            anyhow::bail!("SPT selected CASE absent");
        }
        let close = program.instruction(pc)?;
        if close.identify == CONTROL && close.code == CLOSE {
            pc = close.next;
        }
    }
    anyhow::bail!("SPT CASE search budget")
}
pub(crate) const CONTROL: u32 = 0x66660002;
pub(crate) const OPEN: u32 = 0xa000007b;
pub(crate) const CLOSE: u32 = 0xa000007d;
pub(crate) fn target(program: &Program, pc: u32) -> Result<u32> {
    if pc as usize != program.word_count() {
        program.instruction(pc)?;
    }
    Ok(pc)
}
pub(crate) fn skip_body(program: &Program, pc: u32) -> Result<u32> {
    let body = program.instruction(pc)?;
    target(
        program,
        if body.identify == CONTROL && body.code == OPEN {
            pc.checked_add(*body.operands.first().context("SPT block offset absent")?)
                .context("SPT block offset overflow")?
        } else {
            body.next
        },
    )
}
pub(crate) fn conditional(
    program: &Program,
    pc: u32,
    vars: &mut Variables,
    while_only: bool,
) -> Result<u32> {
    let mut condition = pc;
    for _ in 0..4096 {
        let i = program.instruction(condition)?;
        ensure!(
            i.identify == CONTROL && matches!(i.code, 0x55560002 | 0x55560003 | 0x55560011),
            "SPT conditional record"
        );
        let (value, consumed) = evaluate(i.operands, program, vars)?;
        ensure!(consumed == i.operands.len(), "SPT condition operand span");
        if value > 0 {
            return target(program, i.next);
        }
        let after = skip_body(program, i.next)?;
        if while_only || after as usize == program.word_count() {
            return Ok(after);
        }
        let branch = program.instruction(after)?;
        if branch.identify == CONTROL && branch.code == 0x55560004 {
            return target(program, branch.next);
        }
        if branch.identify != CONTROL || branch.code != 0x55560003 {
            return Ok(after);
        }
        condition = after;
    }
    anyhow::bail!("SPT conditional chain budget")
}
