// SPDX-License-Identifier: MPL-2.0
//! Native VM builtins. Every engine operation is emitted, never rendered here.
pub mod jmp;
pub mod mem;
pub mod obj;
pub mod str;
pub mod sys;
use super::{
    bytecode::Command,
    machine::{Machine, Next},
};
pub fn dispatch(machine: &mut Machine, command: &Command) -> anyhow::Result<Next> {
    match (command.op.modtype, command.op.module) {
        (0, 1 | 5 | 6) => jmp::dispatch(machine, command),
        (1, 10) if command.op.opcode != 100 => str::dispatch(machine, command),
        (1, 11) if !(300..=307).contains(&command.op.opcode) => mem::dispatch(machine, command),
        (1 | 2, 60..=62) => obj::management(machine, command),
        (1 | 2, 71 | 72) => obj::creation(machine, command),
        (1 | 2, 73 | 74) => obj::animation(machine, command),
        (1 | 2, 81 | 82 | 87 | 88 | 90 | 91 | 93 | 94) => obj::properties(machine, command),
        (1 | 2, 84 | 85) => obj::getters(machine, command),
        (1, 4) => match sys::dispatch(machine, command)? {
            Some(next) => Ok(next),
            None => machine.request_command(command),
        },
        _ => machine.request_command(command),
    }
}
