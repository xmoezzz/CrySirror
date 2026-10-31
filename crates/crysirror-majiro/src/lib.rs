// Copyright (c) 2026 CrySirror contributors.
// Native format/opcode research: MajiroTools (C) 2021 Mario Welzig, Robert Jordan.
// MIT; see LICENSE and SOURCE.json.
#![forbid(unsafe_code)]
//! Original Majiro MJO code and semantic boundary. No legacy renderer or mixer.
pub mod machine;
pub mod program;

#[cfg(feature = "runtime")]
pub mod adapter;
pub mod environment;

#[cfg(feature = "runtime")]
mod rng_policy;

#[cfg(feature = "runtime")]
pub mod settings_policy;

#[cfg(feature="runtime")]
pub mod keyboard_policy;
