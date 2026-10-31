// SPDX-License-Identifier: MPL-2.0
//! Original RealLive VM with system calls translated into CrySirror APIs.
#![forbid(unsafe_code)]
pub mod vm;

pub mod adapter;
mod resources;
pub use adapter::{Config, OpticalDiscPolicy, RealLiveFactory, RealLiveVm, register};
