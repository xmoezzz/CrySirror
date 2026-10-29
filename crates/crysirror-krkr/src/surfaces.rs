// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Native plugins whose member surface is known but whose behaviour is not yet
//! mapped onto engine services. Linking installs exactly those members; every
//! one fails visibly when called/read/written, naming plugin and member.
//! Nothing reports false success.
//!
//! Sources: UTF-16 registration strings of the shipped DLLs (inspected
//! 2026-10-28; no code executed), cross-checked with Kirakira c451918
//! `krkr-plugins` reference notes (window_ex.rs, win32_dialog.rs, k2compat.rs).
use crate::natives::{Rt, unsupported};
use crysirror_tjs2::runtime::{Closure, ObjectHandle, Variant};

#[derive(Clone, Copy)]
pub enum Kind {
    Method,
    Property,
}
use Kind::*;

/// `target` is an existing global (`Window`, `System`, `Debug.console`) or,
/// with a `new:` prefix, a class this plugin defines (`new:Window.TouchMouse`).
pub const SURFACES: &[(&str, &str, Kind, &[&str])] = &[
    ("layerstwcopy.dll", "Layer", Method, &["stitchWrappedCopy"]),
    (
        "k2compat.dll",
        "new:Window.TouchMouse",
        Method,
        &[
            "onTouchMouseDown", "onTouchMouseMove", "onTouchMouseUp", "onTouchMouseClick",
            "onTouchMouseDblClick",
        ],
    ),
    ("k2compat.dll", "new:Window.TouchMouse", Property, &["enabled"]),
    ("k2compat.dll", "new:System.ModelessOwnerWindow", Method, &["show", "registerMessageReceiver"]),
    ("k2compat.dll", "new:System.ModelessOwnerWindow", Property, &["stayOnTop"]),
];

pub fn known(name: &str) -> bool {
    SURFACES.iter().any(|(p, ..)| *p == name)
}

fn lookup(rt: &Rt, path: &str) -> Option<ObjectHandle> {
    let mut parts = path.split('.');
    let mut h = rt.global_member(parts.next()?).object_handle()?;
    for p in parts {
        h = rt.object_member(h, p).object_handle()?;
    }
    Some(h)
}

/// A class defined by the plugin; construction succeeds, members fail.
fn define_class(rt: &mut Rt, path: &str) -> Option<ObjectHandle> {
    if let Some(h) = lookup(rt, path) {
        return Some(h);
    }
    let (owner, name) = match path.rsplit_once('.') {
        Some((o, n)) => (Some(lookup(rt, o)?), n.to_owned()),
        None => (None, path.to_owned()),
    };
    let class_name = name.clone();
    let c = rt.alloc_native_constructor(move |rt: &mut Rt, this: Option<ObjectHandle>, _| {
        let h = this
            .map(|h| rt.bound_this(h).unwrap_or(h))
            .filter(|h| *h != rt.global_handle())
            .unwrap_or_else(|| rt.alloc_ordinary_object());
        rt.add_object_class_info(h, &class_name);
        Ok(Variant::Object(h))
    });
    rt.add_object_class_info(c, &name);
    rt.set_object_member(c, &name, Variant::Closure(Closure::new(c, None)));
    rt.register_object_native(c, "finalize", |_: &mut Rt, _, _| Ok(Variant::Void));
    match owner {
        Some(o) => rt.set_object_member(o, &name, Variant::Object(c)),
        None => rt.set_global_member(&name, Variant::Object(c)),
    }
    Some(c)
}

pub fn install(rt: &mut Rt, plugin: &str) {
    for &(p, target, kind, members) in SURFACES {
        if p != plugin {
            continue;
        }
        let h = match target.strip_prefix("new:") {
            Some(path) => define_class(rt, path),
            None => lookup(rt, target),
        };
        let Some(h) = h else { continue };
        let object = target.trim_start_matches("new:");
        for &member in members {
            if !matches!(rt.object_member(h, member), Variant::Void) {
                continue;
            }
            let what = format!("{object}.{member} ({p})");
            let capability = "a mapped engine capability for this native member";
            match kind {
                Method => {
                    rt.register_object_native(h, member, move |_: &mut Rt, _, _| {
                        Err(unsupported(&what, capability))
                    });
                }
                Property => {
                    let w2 = what.clone();
                    rt.register_object_native_property(
                        h,
                        member,
                        move |_, _| Err(unsupported(&what, capability)),
                        move |_, _, _| Err(unsupported(&w2, capability)),
                    );
                }
            }
        }
    }
}
