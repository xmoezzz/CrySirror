// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Native action naming over the shared device-free owner dispatch queue.
use crate::natives::*;
use crysirror_core::deferred_dispatch::DispatchPhase;
use crysirror_tjs2::{TjsError, runtime::Variant};
pub(crate) fn target(handle: usize, kind: u64) -> u64 {
    ((handle as u64) << 2) | kind
}
pub fn install(rt: &mut Rt) {
    let c = native_class(rt, "AsyncTrigger", |rt, t, a| {
        if a.is_empty() {
            return Ok(Variant::Void);
        }
        let h = this_of(rt, t, "AsyncTrigger")?;
        rt.set_object_member(h, "__actionOwner", a[0].clone());
        rt.set_object_member(
            h,
            "__actionName",
            a.get(1)
                .cloned()
                .unwrap_or(Variant::String("action".into())),
        );
        rt.set_object_member(h, "cached", Variant::Integer(1));
        rt.set_object_member(h, "mode", Variant::Integer(0));
        Ok(Variant::Void)
    });
    methods(
        rt,
        c,
        &[
            ("trigger", |rt, t, _| {
                let h = this_of(rt, t, "AsyncTrigger.trigger")?;
                let mode = match rt.object_member(h, "mode").to_integer()? {
                    0 => DispatchPhase::Normal,
                    1 => DispatchPhase::Exclusive,
                    2 => DispatchPhase::Idle,
                    _ => return Err(TjsError::runtime("invalid AsyncTrigger.mode")),
                };
                let cached = rt.object_member(h, "cached").is_truthy();
                let client = rt.host().client;
                rt.host_mut()
                    .deferred
                    .post(client, target(h.0, 0), mode, cached)
                    .map_err(|e| TjsError::runtime(e.to_string()))?;
                Ok(Variant::Void)
            }),
            ("cancel", cancel),
            ("finalize", cancel),
            ("onFire", |rt, t, a| {
                let h = this_of(rt, t, "AsyncTrigger.onFire")?;
                let owner = rt.object_member(h, "__actionOwner");
                let name = rt.object_member(h, "__actionName").to_tjs_string()?;
                if name.is_empty() {
                    rt.call_function(owner, a)
                } else {
                    rt.call_variant_method(owner, &name, a)
                }
            }),
        ],
    );
}
fn cancel(
    rt: &mut Rt,
    t: Option<crysirror_tjs2::runtime::ObjectHandle>,
    _: Vec<Variant>,
) -> crysirror_tjs2::Result<Variant> {
    let h = this_of(rt, t, "AsyncTrigger.cancel")?;
    let client = rt.host().client;
    rt.host_mut().deferred.cancel(client, target(h.0, 0));
    Ok(Variant::Void)
}
