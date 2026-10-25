// SPDX-License-Identifier: MPL-2.0
//! Native VM builtins. Every engine operation is emitted, never rendered here.
pub mod jmp;
pub mod mem;
pub mod str;
use super::{
    bytecode::Command,
    machine::{Machine, Next},
};
pub fn dispatch(machine: &mut Machine, command: &Command) -> anyhow::Result<Next> {
    match (command.op.modtype, command.op.module) {
        (0, 1 | 5 | 6) => jmp::dispatch(machine, command),
        (1, 10) if command.op.opcode != 100 => str::dispatch(machine, command),
        (1, 11) if !(300..=307).contains(&command.op.opcode) => mem::dispatch(machine, command),
        _ => machine.request_command(command),
    }
}
