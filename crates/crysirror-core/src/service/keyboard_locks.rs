// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Per-client logical lock keys. OS import/control is a separate capability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyboardLockState {
    pub caps_lock: bool,
    pub num_lock: bool,
    pub scroll_lock: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyboardLockSnapshot {
    pub client: crate::state::ClientId,
    pub revision: u64,
    pub logical: KeyboardLockState,
    /// No physical state has been imported when None. Never infer physical Off.
    pub physical: Option<KeyboardLockState>,
    pub physical_control_supported: bool,
}
