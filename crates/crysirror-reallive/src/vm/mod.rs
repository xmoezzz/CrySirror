// SPDX-License-Identifier: MPL-2.0
//! Original RealLive interpreter and codecs; no engine implementation or devices.
#![forbid(unsafe_code)]
pub mod archive;
pub mod bytecode;
pub mod clock;
pub mod compression;
pub mod expr;
pub mod gan;
pub mod graphics;
pub mod gameexe;
pub mod machine;
pub mod memory;
pub mod modules;
pub mod nls;
pub mod object;
pub mod opcodes;
pub mod rect;
pub mod scenario;
pub mod serial;
pub mod settings;
pub mod sys_state;
pub use archive::Archive;
pub use gameexe::Gameexe;
pub use machine::{Machine, Next, RunResult, VmEvent};
pub use nls::Nls;
#[cfg(test)]
mod tests;
