// SPDX-License-Identifier: MPL-2.0
//! Original CatSystem2 script policy over CrySirror capability services.
//! No original engine, executable, renderer, mixer or game content is linked.
#![forbid(unsafe_code)]
mod adapter;
pub mod kcs;
mod native_menu;
pub mod scene;
pub mod startup;
mod task_policy;
#[cfg(feature = "runtime")]
pub use adapter::register;
pub use adapter::{CatSystem2Vm, Config};
pub use startup::{parse_startup, Startup};
