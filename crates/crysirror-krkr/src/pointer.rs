// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Coordinate/key naming over logical engine input, and krkr mouse cursors
//! over the shared UI cursor service: the `cr*` constants select standard
//! shapes, cursor files (`.cur`/`.ani`) are decoded by the shared image
//! cursor decoder and defined as client cursors.
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
                // krkrz tTJSNI_BaseLayer::SetCursorByStorage/SetCursorByNumber
                // (LayerIntf.cpp:2710): a string loads a cursor file through
                // storage (TVPGetCursor caches it per placed path and
                // returns its MouseCursor index); a number is a cursor index.
                let v = match v {
                    Variant::String(name) => Variant::Integer(load_cursor(rt, &name.to_string())?),
                    v => Variant::Integer(v.to_integer()?),
                };
                rt.set_object_member(h, "__cursor", v);
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
/// Cursor files loaded through `Layer/Window.cursor = "file"`.
#[derive(Default)]
pub(crate) struct Cursors {
    /// Resolved storage path → krkr cursor index (2, 3, ...).
    by_path: std::collections::BTreeMap<String, i64>,
    /// Last (visible, index) sent to the UI service.
    applied: Option<(bool, i64)>,
}

/// krkrz MouseCursor::AddCursor: custom cursors follow the 24 built-in
/// indices (crSizeAll = -22 .. crHBeam = 1), so the first is 2.
const FIRST_CUSTOM_CURSOR: i64 = 2;
/// The shared UI service keeps at most this many cursors per client.
const MAX_CUSTOM_CURSORS: usize = crysirror_scene::ui::cursor::MAX_CURSORS;

fn load_cursor(rt: &mut Rt, name: &str) -> crysirror_tjs2::Result<i64> {
    let storage = rt.host().storage()?;
    let path = storage
        .resolve(name)
        .ok_or_else(|| TjsError::runtime(format!("cannot load cursor `{name}`: storage not found")))?;
    if let Some(id) = rt.host().cursors.by_path.get(&path) {
        return Ok(*id);
    }
    let bytes = storage
        .read(&path)
        .map_err(|e| TjsError::runtime(format!("cannot load cursor `{name}`: {e}")))?;
    let cursor = crysirror_image::cursor::decode_cursor(&bytes)
        .map_err(|e| TjsError::runtime(format!("cannot load cursor `{name}`: {e:#}")))?;
    let host = rt.host_mut();
    if host.cursors.by_path.len() >= MAX_CUSTOM_CURSORS {
        return Err(TjsError::runtime("custom cursor limit reached"));
    }
    let id = FIRST_CUSTOM_CURSOR + host.cursors.by_path.len() as i64;
    host.cursors.by_path.insert(path, id);
    let frames: Vec<_> = cursor
        .frames
        .iter()
        .map(|f| {
            json!({"width": f.image.width, "height": f.image.height,
                   "hotspot": f.hotspot, "rgba": f.image.rgba})
        })
        .collect();
    let steps: Vec<_> = cursor.steps.iter().map(|(i, ms)| [*i as u32, *ms]).collect();
    host.ui_op(json!({"op": "define_cursor", "id": id, "cursor": {"frames": frames, "steps": steps}}));
    host.cursors.applied = None;
    Ok(id)
}

/// krkrz MouseCursor::CURSOR_INDEXES (win32): the OS shape behind each
/// built-in cursor index. Indices the Windows build shows as the arrow map
/// to the default shape; `None` is crNone (hidden).
fn standard_shape(index: i64) -> Option<&'static str> {
    Some(match index {
        -22 | -5 => "move",
        -21 => "pointer",
        -20 => "help",
        -18 => "not_allowed",
        -11 => "wait",
        -9 => "ew_resize",
        -8 => "nwse_resize",
        -7 => "ns_resize",
        -6 => "nesw_resize",
        -4 => "text",
        -3 => "crosshair",
        -1 => return None,
        // crUpArrow (-10) has no portable shape; like the remaining
        // indices krkrz shows the arrow for, it uses the default.
        _ => "default",
    })
}

pub fn apply(rt: &mut Rt, target: ObjectHandle) {
    let window = rt.host().windows.keys().next().copied().map(ObjectHandle);
    let hidden = window.is_some_and(|w| rt.object_member(w, "__temporary_cursor_hide").is_truthy());
    // tTJSNI_BaseLayer::GetLayerActiveCursor: crDefault (0) defers to the
    // parent layer; the window's own cursor applies when no layer sets one.
    let mut index = 0;
    let mut node = Some(target.0);
    let mut depth = 0;
    while let Some(id) = node {
        if let Variant::Integer(v) = rt.object_member(ObjectHandle(id), "__cursor")
            && v != 0
        {
            index = v;
            break;
        }
        node = rt.host().layers.get(&id).and_then(|l| l.parent);
        depth += 1;
        if depth > 64 {
            break;
        }
    }
    if index == 0
        && let Some(w) = window
        && let Variant::Integer(v) = rt.object_member(w, "__cursor")
    {
        index = v;
    }
    let visible = !hidden && index != -1;
    let host = rt.host_mut();
    if host.cursors.applied == Some((visible, index)) {
        return;
    }
    host.cursors.applied = Some((visible, index));
    host.ui_op(json!({"op":"set_cursor_visible","visible":visible}));
    let custom = index >= FIRST_CUSTOM_CURSOR && host.cursors.by_path.values().any(|v| *v == index);
    let select = if custom {
        json!({"kind": "custom", "id": index})
    } else {
        json!({"kind": "standard", "shape": standard_shape(index).unwrap_or("default")})
    };
    host.ui_op(json!({"op": "set_cursor", "cursor": select}));
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
