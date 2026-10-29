// SPDX-License-Identifier: MPL-2.0
//! RFVP adapter: the vendored portable rfvp core hosted on CrySirror services.
#![forbid(unsafe_code)]
mod adapter;
pub use adapter::{Config, Nls, RfvpVm, register};
