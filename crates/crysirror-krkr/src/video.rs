// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Unloaded native declarations allow stock class construction. Opening media
//! requires prepare/pause/seek semantics beyond the current shared Play/Stop API.
use crate::natives::*;
use crysirror_tjs2::runtime::Variant;
pub fn install(rt: &mut Rt) {
    let c = native_class(rt, "VideoOverlay", |rt, t, _| {
        let h = this_of(rt, t, "VideoOverlay")?;
        for (n, v) in [
            ("mode", 0),
            ("visible", 0),
            ("loop", 0),
            ("frame", 0),
            ("left", 0),
            ("top", 0),
            ("width", 0),
            ("height", 0),
            ("volume", 100000),
            ("pan", 0),
        ] {
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
            // open always fails visibly, so no overlay is ever playing;
            // krkrz Stop without an opened video is a no-op.
            ("stop", |_, _, _| Ok(Variant::Void)),
        ],
    );
    // Geometry/mixing state is recorded; it takes effect once playback maps
    // onto the shared video service (open/play still fail visibly).
    methods(
        rt,
        c,
        &[
            ("setPos", |rt, t, a| {
                let h = this_of(rt, t, "VideoOverlay.setPos")?;
                for (i, k) in ["left", "top"].iter().enumerate() {
                    rt.set_object_member(h, *k, Variant::Integer(int(&a, i, 0)?));
                }
                Ok(Variant::Void)
            }),
            ("setSize", |rt, t, a| {
                let h = this_of(rt, t, "VideoOverlay.setSize")?;
                for (i, k) in ["width", "height"].iter().enumerate() {
                    rt.set_object_member(h, *k, Variant::Integer(int(&a, i, 0)?));
                }
                Ok(Variant::Void)
            }),
            ("setBounds", |rt, t, a| {
                let h = this_of(rt, t, "VideoOverlay.setBounds")?;
                for (i, k) in ["left", "top", "width", "height"].iter().enumerate() {
                    rt.set_object_member(h, *k, Variant::Integer(int(&a, i, 0)?));
                }
                Ok(Variant::Void)
            }),
            ("close", |_, _, _| Ok(Variant::Void)),
            ("pause", |_, _, _| {
                Err(unsupported("VideoOverlay.pause", "native media continuation mapping"))
            }),
            ("rewind", |_, _, _| {
                Err(unsupported("VideoOverlay.rewind", "native media continuation mapping"))
            }),
            ("prepare", |_, _, _| {
                Err(unsupported("VideoOverlay.prepare", "shared prepared media resources"))
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
