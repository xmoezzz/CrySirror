// SPDX-License-Identifier: MPL-2.0
//! Engine-neutral, allocation-capable system services. No renderer or VM dependency.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod binary;
pub mod graphics;
pub mod input_buffer;
pub mod path;
pub mod save;
pub mod service;
pub mod state;
pub mod vfs;
pub mod vm;

use core::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Invalid(&'static str),
    Unsupported(&'static str),
    NotFound,
    ReadOnly,
    Limit(&'static str),
    SnapshotMismatch(&'static str),
    CorruptSnapshot,
    Io(alloc::string::String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(s) => write!(f, "invalid: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
            Self::NotFound => f.write_str("not found"),
            Self::ReadOnly => f.write_str("read-only mount"),
            Self::Limit(s) => write!(f, "limit exceeded: {s}"),
            Self::SnapshotMismatch(s) => write!(f, "snapshot mismatch: {s}"),
            Self::CorruptSnapshot => f.write_str("corrupt snapshot"),
            Self::Io(message) => write!(f, "I/O: {message}"),
        }
    }
}

pub type Result<T> = core::result::Result<T, Error>;

impl core::error::Error for Error {}

pub mod deferred_dispatch;

pub mod region_map;
