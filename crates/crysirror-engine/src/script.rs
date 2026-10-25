// SPDX-License-Identifier: MPL-2.0
//! Script-host contract (ADR 0005).
//!
//! A script host is a [`Lifecycle`] behaviour that runs foreign code and calls
//! back into the engine **only** through the object API on [`EngineContext`].
//! JS/TS is the default host (crate crysirror-script-js). Bytecode VMs that
//! own their own scheduling, waits and saves (Siglus SceneVm, RFVP) are not
//! wrapped here: they stay `VmAdapter` peers inside the runtime scheduler and
//! are visible to the object API as `Client` objects.
use crate::lifecycle::Lifecycle;
use anyhow::Result;

pub trait ScriptHost: Lifecycle {
    /// Stable language id, e.g. "javascript".
    fn language(&self) -> &'static str;
    /// Queue a named source unit. Hosts execute queued units during `on_init`
    /// (or immediately when already initialized), never at construction, so a
    /// host can be built without touching engine state.
    fn add_source(&mut self, name: &str, source: &str) -> Result<()>;
}
