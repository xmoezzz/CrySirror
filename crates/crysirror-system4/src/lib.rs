// Copyright (c) 2026 CrySirror contributors.
// AIN format research: libsys4 (C) 2019 Nunuhara Cabbage, GPL-2.0-or-later.
//! Original AIN program and bounded native VM. No embedded System4 engine or compositor.
//! Shared engine operation mappings remain a separate thin adapter.
#![forbid(unsafe_code)]
pub mod ain;

pub mod instruction;
pub mod opcodes;

pub mod adapter;
pub mod vm;
pub mod worker_adapter;

pub mod sact;
