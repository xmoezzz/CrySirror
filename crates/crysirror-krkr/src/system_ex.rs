// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! PackinOne-bundled systemEx members (wamsoft/systemEx 71182626) and
//! `Layer.fetchImageSize`, which scripts feature-detect.
//!
//! * `System.waitForAppLock(key, timeout)`: the native opens a named mutex
//!   and answers true when none exists or it is released. No other process
//!   shares named mutexes with this host, so the answer is true.
//! * `System.setDpiAwareness(context)`: wraps user32
//!   `SetThreadDpiAwarenessContext`; where that entry is absent the native
//!   logs once and answers void. The host owns DPI handling, so this takes
//!   that branch.
//! * `System.commandExecute(...)`: starts nothing and answers
//!   `%[status:"failed", exitcode:-1, stdout:[]]`.
//! * `Layer.fetchImageSize(storage)`: `[width, height]` from the image
//!   header without loading it into a layer.
use crate::natives::{Rt, string};
use crysirror_tjs2::{Result, TjsError, runtime::{NativeArgCount, Variant}};

pub(crate) fn install(rt: &mut Rt) {
    if let Some(system) = rt.global_member("System").object_handle() {
        rt.register_object_native_with_arg_count(system, "waitForAppLock", NativeArgCount::AtLeast(1), |_: &mut Rt, _, _| {
            Ok(Variant::Integer(1))
        });
        rt.register_object_native(system, "setDpiAwareness", |rt: &mut Rt, _, a: Vec<Variant>| {
            if !a.is_empty() && !matches!(rt.object_member(rt.global_handle(), "__dpi_logged"), Variant::Integer(1)) {
                let global = rt.global_handle();
                rt.set_object_member(global, "__dpi_logged", Variant::Integer(1));
                rt.host_mut().push_log("SetThreadDpiAwarenessContext not found.".into());
            }
            Ok(Variant::Void)
        });
        // No external process is ever started (owner decision): the call
        // answers the native's failure record.
        rt.register_object_native(system, "commandExecute", |rt: &mut Rt, _, _| {
            let d = rt.alloc_dictionary_object();
            rt.set_object_member(d, "status", Variant::String("failed".into()));
            rt.set_object_member(d, "exitcode", Variant::Integer(-1));
            let lines = rt.alloc_array_object(vec![]);
            rt.set_object_member(d, "stdout", Variant::Object(lines));
            Ok(Variant::Object(d))
        });
        for (name, value) in [("dacUnaware", -1), ("dacSystemAware", -2), ("dacPerMonitorAware", -3), ("dacPerMonitorAwareV2", -4), ("dacUnawareGdiScaled", -5)] {
            rt.set_object_member(system, name, Variant::Integer(value));
        }
    }
    if let Some(layer) = rt.global_member("Layer").object_handle() {
        rt.register_object_native_with_arg_count(layer, "fetchImageSize", NativeArgCount::AtLeast(1), |rt: &mut Rt, _, a: Vec<Variant>| {
            let name = string(&a, 0, "Layer.fetchImageSize")?;
            rt.host_mut().before_image_work()?;
            let storage = rt.host().storage()?;
            let path = storage
                .resolve_with(&name, &[".png", ".bmp", ".jpg", ".jpeg", ".tlg", ".webp"])
                .ok_or_else(|| TjsError::runtime(format!("Layer.fetchImageSize: storage {name:?} not found")))?;
            let bytes = storage.read(&path).map_err(|e| TjsError::runtime(e.to_string()))?;
            let [w, h] = crysirror_image::encoded_dimensions_hint(&bytes)
                .ok_or_else(|| TjsError::runtime(format!("Layer.fetchImageSize: {name:?} has no readable image header")))?;
            let arr = rt.alloc_array_object(vec![Variant::Integer(w.into()), Variant::Integer(h.into())]);
            Ok(Variant::Object(arr))
        });
    }
}

/// `Plugins.link("PackinOne.dll")`: the script-defined `SliceLayer` class
/// (the DLL carries it as embedded TJS; this is an original source with the
/// same members, see `slice_layer.tjs`).
pub(crate) fn install_slice_layer(rt: &mut Rt) -> Result<()> {
    if rt.global_member("SliceLayer").object_handle().is_some() {
        return Ok(());
    }
    crate::natives::run_source(rt, "PackinOne:SliceLayer.tjs", include_str!("slice_layer.tjs"), None)?;
    Ok(())
}
