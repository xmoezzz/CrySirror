// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Shared logical input lock facade, independent of any native VM.
use crate::{EngineContext, Value};
use anyhow::{ensure, Context, Result};
use crysirror_core::service::{HostServices, KeyboardLockSnapshot};
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    #[serde(default)]
    caps_lock: Option<bool>,
    #[serde(default)]
    num_lock: Option<bool>,
    #[serde(default)]
    scroll_lock: Option<bool>,
    #[serde(default)]
    mode: Option<String>,
}
fn output(s: KeyboardLockSnapshot) -> Value {
    Value::from_json(
        serde_json::json!({"client":s.client.0,"revision":s.revision.to_string(),"logical":{"caps_lock":s.logical.caps_lock,"num_lock":s.logical.num_lock,"scroll_lock":s.logical.scroll_lock},"physical":s.physical.map(|v|serde_json::json!({"caps_lock":v.caps_lock,"num_lock":v.num_lock,"scroll_lock":v.scroll_lock})),"physical_control_supported":s.physical_control_supported}),
    )
}
pub(crate) fn call(cx: &mut EngineContext, method: &str, args: &[Value]) -> Result<Option<Value>> {
    if !matches!(method, "keyboardLocks" | "setKeyboardLocks") {
        return Ok(None);
    }
    let s = cx.state()?;
    let mut snapshot = s
        .runtime
        .host
        .keyboard_locks(s.client)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if method == "setKeyboardLocks" {
        let patch: Patch = serde_json::from_value(
            args.first()
                .context("keyboard lock state required")?
                .to_json(),
        )?;
        ensure!(
            patch.mode.as_deref().is_none_or(|m| m == "logical"),
            "physical keyboard control is not supported"
        );
        if let Some(v) = patch.caps_lock {
            snapshot.logical.caps_lock = v;
        }
        if let Some(v) = patch.num_lock {
            snapshot.logical.num_lock = v;
        }
        if let Some(v) = patch.scroll_lock {
            snapshot.logical.scroll_lock = v;
        }
        snapshot = s
            .runtime
            .host
            .set_keyboard_locks(s.client, snapshot.logical)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    Ok(Some(output(snapshot)))
}
