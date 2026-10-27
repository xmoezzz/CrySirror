// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Preserve original builtin serializers; only file capability/commit boundaries
//! are adapted. A successful native return follows the atomic worker commit.
use crate::natives::*;
use krkr_tjs2::runtime::{Closure, Variant};
pub fn install(rt: &mut Rt) {
    for name in ["Dictionary", "Array"] {
        let Some(class) = rt.global_member(name).object_handle() else {
            continue;
        };
        let original = rt.object_member(class, "saveStruct");
        let Some(function) = original.object_handle() else {
            continue;
        };
        rt.set_object_member(class, "__crysirror_original_saveStruct", original);
        rt.register_object_native(class, "saveStruct", move |rt: &mut Rt, t, a| {
            if !rt.host().can_suspend {
                return Err(unsupported(
                    "saveStruct in this callback",
                    "native file continuation boundary",
                ));
            }
            let target = this_of(rt, t, "saveStruct")?;
            rt.host_mut().io_barrier = true;
            let result =
                rt.call_function(Variant::Closure(Closure::new(function, Some(target))), a);
            rt.host_mut().io_barrier = false;
            let value = result?;
            if !rt.host().writes.is_empty() {
                rt.request_suspend();
            }
            Ok(value)
        });
    }
}
pub fn poll(rt: &mut Rt) -> anyhow::Result<bool> {
    use crysirror_core::service::StorageWriteState;
    while let Some(write) = rt.host_mut().writes.front_mut() {
        match write.poll()? {
            StorageWriteState::Pending => return Ok(false),
            StorageWriteState::Ready => {
                rt.host_mut().writes.pop_front();
            }
            StorageWriteState::Failed(e) => anyhow::bail!("native atomic write failed: {e}"),
        }
    }
    Ok(true)
}
