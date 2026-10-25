// SPDX-License-Identifier: MPL-2.0
//! Original RealLive interpreter and codecs; no engine implementation or devices.
#![forbid(unsafe_code)]
pub mod archive;
pub mod bytecode;
pub mod compression;
pub mod expr;
pub mod gameexe;
pub mod machine;
pub mod memory;
pub mod modules;
pub mod nls;
pub mod opcodes;
pub mod scenario;
pub mod serial;
pub use archive::Archive;
pub use gameexe::Gameexe;
pub use machine::{Machine, Next, RunResult, VmEvent};
pub use nls::Nls;
#[cfg(test)]
mod tests;
