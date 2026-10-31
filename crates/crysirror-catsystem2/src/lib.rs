// SPDX-License-Identifier: MPL-2.0
//! Original CatSystem2 script policy over CrySirror capability services.
//! No original engine, executable, renderer, mixer or game content is linked.
#![forbid(unsafe_code)]
mod adapter;
pub mod kcs;
mod native_menu;
mod native_values;
mod native_interpreter;
mod sound_definition;
mod native_save;
mod image_list;
mod fes_task;
pub mod scene;
pub mod startup;
mod task_policy;
#[cfg(feature = "runtime")]
pub use adapter::register;
pub use adapter::{CatSystem2Vm, Config};
pub use startup::{parse_startup, Startup};

mod loop_policy;

mod native_settings;
