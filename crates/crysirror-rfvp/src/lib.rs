// SPDX-License-Identifier: MPL-2.0
//! RFVP semantic adapter over CrySirror resources, rendering and audio.
#![forbid(unsafe_code)]
mod adapter;
pub use adapter::{register, Config, Nls, RfvpVm};
