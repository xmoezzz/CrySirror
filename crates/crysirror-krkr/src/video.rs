// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Unloaded native declarations allow stock class construction. Opening media
//! requires prepare/pause/seek semantics beyond the current shared Play/Stop API.
use crate::natives::*;
use krkr_tjs2::runtime::Variant;
pub fn install(rt: &mut Rt) {
    let c = native_class(rt, "VideoOverlay", |rt, t, _| {
        let h = this_of(rt, t, "VideoOverlay")?;
        for (n, v) in [("mode", 0), ("visible", 0), ("loop", 0), ("frame", 0)] {
            rt.set_object_member(h, n, Variant::Integer(v));
        }
        Ok(Variant::Void)
    });
    methods(
        rt,
        c,
        &[
            ("finalize", |_, _, _| Ok(Variant::Void)),
            ("open", |_, _, _| {
                Err(unsupported(
                    "VideoOverlay.open",
                    "shared prepared media resources and native playback policy",
                ))
            }),
            ("play", |_, _, _| {
                Err(unsupported(
                    "VideoOverlay.play",
                    "native media continuation mapping",
                ))
            }),
            ("stop", |_, _, _| {
                Err(unsupported(
                    "VideoOverlay.stop",
                    "native media continuation mapping",
                ))
            }),
        ],
    );
    rt.register_object_native_property(
        c,
        "status",
        |_, _| Ok(Variant::String("unload".into())),
        read_only,
    );
}
