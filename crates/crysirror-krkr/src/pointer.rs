// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Coordinate/key naming over logical engine input. This profile supports the
//! host default arrow and hidden cursor; custom cursor resources fail visibly.
use crate::natives::*;
use crysirror_tjs2::{
    TjsError,
    runtime::{ObjectHandle, Variant},
};
use serde_json::json;
pub(crate) const KEYS: &[(u64, i64)] = &[
    (crysirror_core::service::buttons::ENTER, 13),
    (crysirror_core::service::buttons::SPACE, 32),
    (crysirror_core::service::buttons::LEFT, 37),
    (crysirror_core::service::buttons::RIGHT, 39),
    (crysirror_core::service::buttons::UP, 38),
    (crysirror_core::service::buttons::DOWN, 40),
    (crysirror_core::service::buttons::TAB, 9),
    (crysirror_core::service::buttons::ESCAPE, 27),
    (crysirror_core::service::buttons::BACKSPACE, 8),
    (crysirror_core::service::buttons::SHIFT, 16),
    (crysirror_core::service::buttons::CTRL, 17),
    (crysirror_core::service::buttons::HOME, 36),
    (crysirror_core::service::buttons::END, 35),
    (crysirror_core::service::buttons::PAGE_UP, 33),
    (crysirror_core::service::buttons::PAGE_DOWN, 34),
    (crysirror_core::service::buttons::KEY_W, 87),
    (crysirror_core::service::buttons::KEY_A, 65),
    (crysirror_core::service::buttons::KEY_S, 83),
    (crysirror_core::service::buttons::KEY_D, 68),
    (crysirror_core::service::buttons::KEY_E, 69),
    (crysirror_core::service::buttons::KEY_Q, 81),
    (crysirror_core::service::buttons::KEY_F, 70),
    (crysirror_core::service::buttons::KEY_R, 82),
    (crysirror_core::service::buttons::KEY_0, 48),
    (crysirror_core::service::buttons::KEY_1, 49),
    (crysirror_core::service::buttons::KEY_2, 50),
    (crysirror_core::service::buttons::KEY_3, 51),
    (crysirror_core::service::buttons::KEY_4, 52),
    (crysirror_core::service::buttons::KEY_5, 53),
    (crysirror_core::service::buttons::KEY_6, 54),
    (crysirror_core::service::buttons::KEY_7, 55),
    (crysirror_core::service::buttons::KEY_8, 56),
    (crysirror_core::service::buttons::KEY_9, 57),
    (crysirror_core::service::buttons::F1, 112),
    (crysirror_core::service::buttons::F2, 113),
    (crysirror_core::service::buttons::F3, 114),
    (crysirror_core::service::buttons::F4, 115),
    (crysirror_core::service::buttons::F5, 116),
    (crysirror_core::service::buttons::F6, 117),
    (crysirror_core::service::buttons::F7, 118),
    (crysirror_core::service::buttons::F8, 119),
    (crysirror_core::service::buttons::F9, 120),
    (crysirror_core::service::buttons::F10, 121),
    (crysirror_core::service::buttons::F11, 122),
    (crysirror_core::service::buttons::F12, 123),
];
pub fn install(rt: &mut Rt) {
    for name in ["Window", "Layer"] {
        let Some(c) = rt.global_member(name).object_handle() else {
            continue;
        };
        rt.register_object_native_property(
            c,
            "cursor",
            |rt, t| {
                let h = this_of(rt, t, "cursor")?;
                Ok(match rt.object_member(h, "__cursor") {
                    Variant::Void => Variant::Integer(0),
                    v => v,
                })
            },
            |rt, t, v| {
                let h = this_of(rt, t, "cursor")?;
                // Standard krkr cursor constants (crDefault..crHandPoint) are
                // kept per layer; the host does not yet switch its cursor
                // shape on hover. Cursor resources (strings) need a shared
                // cursor-image service and fail visibly.
                if matches!(v, Variant::String(_)) {
                    return Err(unsupported("cursor resource", "shared custom cursor image mapping"));
                }
                let v = v.to_integer()?;
                if !(-22..=0).contains(&v) {
                    return Err(unsupported(
                        "cursor shape",
                        "shared custom cursor shape/resource mapping",
                    ));
                }
                rt.set_object_member(h, "__cursor", Variant::Integer(v));
                Ok(())
            },
        );
        // krkrz Layer.setCursorPos(x, y): move the pointer to a layer-local
        // position. The engine asks the presenting host to warp the OS
        // pointer; cursorX/cursorY read the new position at once.
        methods(
            rt,
            c,
            &[("setCursorPos", |rt, t, a| {
                let h = this_of(rt, t, "Layer.setCursorPos")?;
                let (mut x, mut y) = (need_int(&a, 0, "Layer.setCursorPos")?, need_int(&a, 1, "Layer.setCursorPos")?);
                let mut node = Some(h.0);
                let mut depth = 0;
                while let Some(id) = node {
                    let l = rt.host().layer(ObjectHandle(id))?;
                    x += l.left;
                    y += l.top;
                    node = l.parent;
                    depth += 1;
                    if depth > 64 {
                        return Err(TjsError::runtime("cursor ancestry limit"));
                    }
                }
                warp(rt, x, y)
            })],
        );
        for (field, axis) in [("cursorX", 0), ("cursorY", 1)] {
            rt.register_object_native_property(
                c,
                field,
                move |rt, t| {
                    let h = this_of(rt, t, "cursor position")?;
                    let f = rt.host().input;
                    let mut v = if axis == 0 {
                        f.pointer_x as i64
                    } else {
                        f.pointer_y as i64
                    };
                    let mut node = Some(h.0);
                    let mut depth = 0;
                    while let Some(id) = node {
                        let Some(l) = rt.host().layers.get(&id) else {
                            break;
                        };
                        v -= if axis == 0 { l.left } else { l.top };
                        node = l.parent;
                        depth += 1;
                        if depth > 64 {
                            return Err(TjsError::runtime("cursor ancestry limit"));
                        }
                    }
                    Ok(Variant::Integer(v))
                },
                read_only,
            );
        }
    }
    if let Some(system) = rt.global_member("System").object_handle() {
        methods(
            rt,
            system,
            &[("getKeyState", |rt, _, a| {
                let key = need_int(&a, 0, "virtual key")?;
                let mask = match key {
                    1 => crysirror_core::service::buttons::MOUSE_LEFT,
                    2 => crysirror_core::service::buttons::MOUSE_RIGHT,
                    437..=479 => 0,
                    _ => KEYS
                        .iter()
                        .find(|(_, code)| *code == key)
                        .map(|(mask, _)| *mask)
                        .ok_or_else(|| {
                            unsupported("System.getKeyState", "this native virtual key mapping")
                        })?,
                };
                Ok(Variant::Integer(
                    (rt.host().input.buttons_down & mask != 0) as i64,
                ))
            })],
        );
    }
}
pub fn apply(rt: &mut Rt, target: ObjectHandle) {
    if let Some(w) = rt.host().windows.keys().next().copied() {
        if rt
            .object_member(ObjectHandle(w), "__temporary_cursor_hide")
            .is_truthy()
        {
            rt.host_mut()
                .ui_op(json!({"op":"set_cursor_visible","visible":false}));
            return;
        }
    }
    let mut shape = rt.object_member(target, "__cursor");
    if matches!(shape, Variant::Void | Variant::Integer(0)) {
        if let Some(w) = rt.host().windows.keys().next().copied() {
            shape = rt.object_member(ObjectHandle(w), "__cursor");
        }
    }
    rt.host_mut()
        .ui_op(json!({"op":"set_cursor_visible","visible":!matches!(shape,Variant::Integer(-1))}));
}

/// Request an OS pointer warp to canvas `(x, y)` and report it immediately.
fn warp(rt: &mut Rt, x: i64, y: i64) -> crysirror_tjs2::Result<Variant> {
    let (x, y) = (x.clamp(-65536, 65536) as i32, y.clamp(-65536, 65536) as i32);
    let host = rt.host_mut();
    host.input.pointer_x = x;
    host.input.pointer_y = y;
    host.ui_op(serde_json::json!({"op": "warp_pointer", "position": [x, y]}));
    Ok(Variant::Void)
}
