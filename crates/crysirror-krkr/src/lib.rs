// SPDX-License-Identifier: AGPL-3.0-or-later
//! Optional KiriKiri (吉里吉里) adapter (ADR 0053).
//!
//! Original TJS2 scripts run on the vendored Kirakira VM (`krkr-tjs2`,
//! AGPL-3.0-or-later). KiriKiri's native classes are translated onto engine
//! Layer/Canvas services, the engine clock and capability storage; there is
//! no KiriKiri renderer, compositor or mixer in this crate. Because of the
//! upstream licence this crate is not part of the default desktop build.
#![forbid(unsafe_code)]
// TjsError is the vendored VM's error type; its size is not ours to change.
#![allow(clippy::result_large_err)]
pub mod adapter;
mod constants;
pub mod host;
pub mod natives;
pub mod storage;

pub use adapter::{Config, KrkrFactory, KrkrVm, register};
