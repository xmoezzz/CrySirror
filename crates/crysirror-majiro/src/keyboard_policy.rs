// Copyright (c) 2026 CrySirror contributors. MIT; see LICENSE/SOURCE.json.
//! Native fast-mode policy over an explicitly logical engine keyboard.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardLockPolicy {
    /// Uses the engine client's own logical lock state. Does not import/toggle OS.
    #[default]
    EngineLogical,
    /// Physical Windows lock-key import/control is not implemented.
    PhysicalWindows,
}
