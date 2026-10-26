// Copyright (c) 2026 CrySirror contributors (CrySirror adapter).
// Native MenuItem conventions inspected in Kirakira c451918023477620ff9055a2eb8a6714b947d9c1,
// native/classes.rs; original Kirakira authors retain their copyright.
// This file is distributed under the GNU Affero General Public License,
// version 3 or (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Native handles/properties only; layout, radio groups, drawing and input are
//! reusable engine MenuSpec/UiSystem operations, independent of this VM.
use crate::{
    host::KrkrHost,
    natives::{Rt, methods, native_class, need_int, properties, read_only, this_of, unsupported},
};
use crysirror_scene::ui::menu::{MenuItemSpec, MenuSpec};
use krkr_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
use serde_json::{Value, json};
#[derive(Clone)]
pub struct MenuState {
    pub id: u32,
    pub window: usize,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    caption: String,
    checked: bool,
    enabled: bool,
    visible: bool,
    radio: bool,
    group: u32,
}
fn state(rt: &Rt, h: ObjectHandle) -> Result<&MenuState> {
    rt.host()
        .menus
        .get(&h.0)
        .ok_or_else(|| TjsError::runtime("MenuItem is not constructed"))
}
fn state_mut(rt: &mut Rt, h: ObjectHandle) -> Result<&mut MenuState> {
    rt.host_mut()
        .menus
        .get_mut(&h.0)
        .ok_or_else(|| TjsError::runtime("MenuItem is not constructed"))
}
fn make(rt: &mut Rt, h: ObjectHandle, window: usize, caption: String) -> Result<()> {
    if rt.host().menus.contains_key(&h.0) {
        return Ok(());
    }
    if !rt.host().windows.contains_key(&window) {
        return Err(TjsError::runtime("MenuItem owner is not an engine Window"));
    }
    let id = (1..=1024)
        .find(|id| !rt.host().menus.values().any(|m| m.id == *id))
        .ok_or_else(|| TjsError::runtime("native menu limit"))?;
    rt.host_mut().menus.insert(
        h.0,
        MenuState {
            id,
            window,
            parent: None,
            children: vec![],
            caption,
            checked: false,
            enabled: true,
            visible: true,
            radio: false,
            group: 0,
        },
    );
    let children = rt.alloc_array_object(vec![]);
    rt.set_object_member(h, "__crysirror_menu_children", Variant::Object(children));
    rt.set_object_member(
        h,
        "__crysirror_menu_window",
        Variant::Object(ObjectHandle(window)),
    );
    Ok(())
}
pub fn root(rt: &mut Rt, window: ObjectHandle) -> Result<ObjectHandle> {
    let class = rt
        .global_member("MenuItem")
        .object_handle()
        .ok_or_else(|| TjsError::runtime("MenuItem class unavailable"))?;
    let h = rt.alloc_ordinary_object();
    rt.set_object_super_class(h, class);
    rt.add_object_class_info(h, "MenuItem");
    make(rt, h, window.0, String::new())?;
    rt.set_object_member(window, "__crysirror_menu_root", Variant::Object(h));
    Ok(h)
}
fn children_array(rt: &Rt, h: ObjectHandle) -> Result<ObjectHandle> {
    rt.object_member(h, "__crysirror_menu_children")
        .object_handle()
        .ok_or_else(|| TjsError::runtime("menu children missing"))
}
fn detach(rt: &mut Rt, child: ObjectHandle) -> Result<()> {
    let parent = state(rt, child)?.parent;
    if let Some(parent) = parent {
        state_mut(rt, ObjectHandle(parent))?
            .children
            .retain(|c| *c != child.0);
        let array = children_array(rt, ObjectHandle(parent))?;
        rt.array_remove_value(array, &Variant::Object(child));
    }
    state_mut(rt, child)?.parent = None;
    Ok(())
}
fn attach(rt: &mut Rt, parent: ObjectHandle, child: ObjectHandle, index: usize) -> Result<()> {
    let window = state(rt, parent)?.window;
    if state(rt, child)?.window != window {
        return Err(TjsError::runtime("menu items belong to different windows"));
    }
    let mut ancestor = Some(parent.0);
    let mut depth = 0;
    while let Some(h) = ancestor {
        if h == child.0 {
            return Err(TjsError::runtime("menu parent cycle"));
        }
        ancestor = state(rt, ObjectHandle(h))?.parent;
        depth += 1;
        if depth > 16 {
            return Err(TjsError::runtime("menu nesting limit"));
        }
    }
    detach(rt, child)?;
    let index = index.min(state(rt, parent)?.children.len());
    state_mut(rt, parent)?.children.insert(index, child.0);
    state_mut(rt, child)?.parent = Some(parent.0);
    let array = children_array(rt, parent)?;
    rt.array_insert(array, index, Variant::Object(child));
    Ok(())
}
fn checked(rt: &mut Rt, h: ObjectHandle, value: bool) -> Result<()> {
    let siblings = state(rt, h)?
        .parent
        .and_then(|p| rt.host().menus.get(&p))
        .map(|m| m.children.clone())
        .unwrap_or(vec![h.0]);
    let mut items = siblings
        .iter()
        .map(|id| item(rt.host(), *id, 0))
        .collect::<Result<Vec<_>>>()?;
    MenuItemSpec::set_checked(&mut items, state(rt, h)?.id, value);
    fn apply(host: &mut KrkrHost, items: &[MenuItemSpec]) {
        for item in items {
            if let Some(m) = host.menus.values_mut().find(|m| m.id == item.id) {
                m.checked = item.checked;
            }
            apply(host, &item.children);
        }
    }
    apply(rt.host_mut(), &items);
    Ok(())
}
fn label(caption: &str) -> String {
    let mut caption = caption.to_owned();
    for letter in ('A'..='Z').chain('a'..='z').chain('0'..='9') {
        caption = caption.replace(&format!("(&{letter})"), "");
    }
    let mut out = String::new();
    let mut chars = caption.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                out.push('&');
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn item(host: &KrkrHost, h: usize, depth: u8) -> Result<MenuItemSpec> {
    if depth > 16 {
        return Err(TjsError::runtime("menu nesting limit"));
    }
    let m = host
        .menus
        .get(&h)
        .ok_or_else(|| TjsError::runtime("menu reference invalidated"))?;
    Ok(MenuItemSpec {
        id: m.id,
        label: label(&m.caption),
        visible: m.visible,
        enabled: m.enabled,
        checked: m.checked,
        radio: m.radio,
        group: m.group,
        separator: m.caption == "-",
        children: m
            .children
            .iter()
            .map(|h| item(host, *h, depth + 1))
            .collect::<Result<_>>()?,
    })
}
fn spec(host: &KrkrHost, root: usize, context: bool) -> Result<MenuSpec> {
    let m = host
        .menus
        .get(&root)
        .ok_or_else(|| TjsError::runtime("menu root missing"))?;
    Ok(MenuSpec {
        id: m.id + if context { 32768 } else { 0 },
        rect: [0., 0., host.canvas[0] as f32, 24.],
        font: host.font.clone(),
        text_size: 18,
        cell_width: 100.,
        popup_width: 260.,
        row_height: 24.,
        visible: m.visible && host.windows.get(&m.window).is_some_and(|w| w.visible),
        context,
        items: m
            .children
            .iter()
            .map(|h| item(host, *h, 0))
            .collect::<Result<_>>()?,
    })
}
pub fn sync(host: &mut KrkrHost) -> Result<()> {
    let specs = host
        .windows
        .values()
        .filter_map(|w| w.menu)
        .map(|root| spec(host, root, false))
        .collect::<Result<Vec<_>>>();
    let specs = specs?;
    let value = json!(specs);
    if host.menus_last.as_ref() != Some(&value) {
        let old = host
            .menus_last
            .as_ref()
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for old in old {
            if !specs.iter().any(|s| old["id"] == s.id) {
                host.ui_op(json!({"op":"remove_menu","id":old["id"]}));
            }
        }
        for spec in &specs {
            host.ui_op(json!({"op":"set_menu","menu":spec}));
        }
        host.menus_last = Some(value);
    }
    for (root, position) in std::mem::take(&mut host.menu_popups) {
        let spec = spec(host, root, true)?;
        let id = spec.id;
        host.ui_op(json!({"op":"set_menu","menu":spec}));
        host.ui_op(json!({"op":"open_menu","id":id,"position":position}));
    }
    Ok(())
}
pub fn dispatch(rt: &mut Rt, bytes: &[u8]) -> Result<()> {
    let events: Vec<crysirror_scene::ui::UiEvent> =
        serde_json::from_slice(bytes).map_err(|e| TjsError::runtime(e.to_string()))?;
    for event in events {
        let h = rt
            .host()
            .menus
            .iter()
            .find(|(_, m)| m.id == event.widget.id)
            .map(|(h, _)| ObjectHandle(*h));
        if let Some(h) = h {
            checked(rt, h, event.value > 0.5)?;
            rt.call_object_method(h, "onClick", vec![])?;
        }
    }
    Ok(())
}
pub fn install(rt: &mut Rt) {
    let class = native_class(rt, "MenuItem", |rt, this, args| {
        let h = this_of(rt, this, "MenuItem")?;
        // The VM invokes native bases without arguments before script subclass initialization.
        if args.is_empty() {
            return Ok(Variant::Void);
        }
        let owner = args
            .first()
            .and_then(Variant::object_handle)
            .ok_or_else(|| TjsError::runtime("MenuItem owner required"))?;
        let owner = rt.bound_this(owner).unwrap_or(owner);
        make(
            rt,
            h,
            owner.0,
            args.get(1)
                .map(Variant::to_tjs_string)
                .transpose()?
                .unwrap_or_default(),
        )?;
        Ok(Variant::Void)
    });
    methods(
        rt,
        class,
        &[
            ("finalize", |_, _, _| Ok(Variant::Void)), // Native base finalizer is explicitly empty.
            ("onClick", |_, _, _| Ok(Variant::Void)), // Default event handler; script subclasses override it.
            ("click", |rt, this, _| {
                let h = this_of(rt, this, "MenuItem.click")?;
                if state(rt, h)?.enabled {
                    rt.call_object_method(h, "onClick", vec![])?;
                }
                Ok(Variant::Void)
            }),
            ("add", |rt, this, args| {
                let h = this_of(rt, this, "MenuItem.add")?;
                let child = args
                    .first()
                    .and_then(Variant::object_handle)
                    .ok_or_else(|| TjsError::runtime("MenuItem.add child required"))?;
                let child = rt.bound_this(child).unwrap_or(child);
                attach(rt, h, child, usize::MAX)?;
                Ok(Variant::Object(child))
            }),
            ("insert", |rt, this, args| {
                let h = this_of(rt, this, "MenuItem.insert")?;
                let child = args
                    .first()
                    .and_then(Variant::object_handle)
                    .ok_or_else(|| TjsError::runtime("MenuItem.insert child required"))?;
                let child = rt.bound_this(child).unwrap_or(child);
                let index = need_int(&args, 1, "MenuItem.insert")?.max(0) as usize;
                attach(rt, h, child, index)?;
                Ok(Variant::Object(child))
            }),
            ("remove", |rt, this, args| {
                let parent = this_of(rt, this, "MenuItem.remove")?;
                let child = args
                    .first()
                    .and_then(Variant::object_handle)
                    .ok_or_else(|| TjsError::runtime("MenuItem.remove child required"))?;
                let child = rt.bound_this(child).unwrap_or(child);
                if state(rt, child)?.parent != Some(parent.0) {
                    return Err(TjsError::runtime(
                        "MenuItem.remove child belongs to another parent",
                    ));
                }
                detach(rt, child)?;
                Ok(Variant::Void)
            }),
            ("clear", |rt, this, _| {
                let h = this_of(rt, this, "MenuItem.clear")?;
                for child in state(rt, h)?.children.clone() {
                    detach(rt, ObjectHandle(child))?;
                }
                Ok(Variant::Void)
            }),
            ("popup", |_, _, _| {
                Err(unsupported(
                    "MenuItem.popup",
                    "native synchronous popup continuation; use the engine UI.openMenu API for asynchronous menus",
                ))
            }),
        ],
    );
    properties(
        rt,
        class,
        &[
            (
                "caption",
                |rt, this| {
                    let h = this_of(rt, this, "MenuItem.caption")?;
                    Ok(Variant::String(state(rt, h)?.caption.clone()))
                },
                |rt, this, v| {
                    let h = this_of(rt, this, "MenuItem.caption")?;
                    let caption = v.to_tjs_string()?;
                    if caption.len() > 4096 || caption.contains(['\0', '\n', '\r']) {
                        return Err(TjsError::runtime("menu caption limit"));
                    }
                    state_mut(rt, h)?.caption = caption;
                    Ok(())
                },
            ),
            (
                "checked",
                |rt, this| {
                    let h = this_of(rt, this, "MenuItem.checked")?;
                    Ok(Variant::Integer(state(rt, h)?.checked as i64))
                },
                |rt, this, v| {
                    let h = this_of(rt, this, "MenuItem.checked")?;
                    checked(rt, h, v.is_truthy())
                },
            ),
            (
                "visible",
                |rt, this| {
                    let h = this_of(rt, this, "MenuItem.visible")?;
                    Ok(Variant::Integer(state(rt, h)?.visible as i64))
                },
                |rt, this, v| {
                    let h = this_of(rt, this, "MenuItem.visible")?;
                    state_mut(rt, h)?.visible = v.is_truthy();
                    Ok(())
                },
            ),
            (
                "enabled",
                |rt, this| {
                    let h = this_of(rt, this, "MenuItem.enabled")?;
                    Ok(Variant::Integer(state(rt, h)?.enabled as i64))
                },
                |rt, this, v| {
                    let h = this_of(rt, this, "MenuItem.enabled")?;
                    state_mut(rt, h)?.enabled = v.is_truthy();
                    Ok(())
                },
            ),
            (
                "radio",
                |rt, this| {
                    let h = this_of(rt, this, "MenuItem.radio")?;
                    Ok(Variant::Integer(state(rt, h)?.radio as i64))
                },
                |rt, this, v| {
                    let h = this_of(rt, this, "MenuItem.radio")?;
                    state_mut(rt, h)?.radio = v.is_truthy();
                    if state(rt, h)?.checked {
                        checked(rt, h, true)?;
                    }
                    Ok(())
                },
            ),
            (
                "group",
                |rt, this| {
                    let h = this_of(rt, this, "MenuItem.group")?;
                    Ok(Variant::Integer(state(rt, h)?.group as i64))
                },
                |rt, this, v| {
                    let h = this_of(rt, this, "MenuItem.group")?;
                    state_mut(rt, h)?.group = u32::try_from(v.to_integer()?)
                        .map_err(|_| TjsError::runtime("menu group"))?;
                    if state(rt, h)?.checked {
                        checked(rt, h, true)?;
                    }
                    Ok(())
                },
            ),
            (
                "parent",
                |rt, this| {
                    let h = this_of(rt, this, "MenuItem.parent")?;
                    Ok(state(rt, h)?
                        .parent
                        .map(|h| Variant::Object(ObjectHandle(h)))
                        .unwrap_or_default())
                },
                read_only,
            ),
            (
                "children",
                |rt, this| {
                    let h = this_of(rt, this, "MenuItem.children")?;
                    let array = children_array(rt, h)?;
                    let children = state(rt, h)?.children.clone();
                    rt.array_clear(array);
                    for child in children {
                        rt.array_push(array, Variant::self_bound(ObjectHandle(child)));
                    }
                    Ok(Variant::Object(array))
                },
                read_only,
            ),
            (
                "owner",
                |rt, this| {
                    let h = this_of(rt, this, "MenuItem.owner")?;
                    Ok(Variant::Object(ObjectHandle(state(rt, h)?.window)))
                },
                read_only,
            ),
            (
                "index",
                |rt, this| {
                    let h = this_of(rt, this, "MenuItem.index")?;
                    let m = state(rt, h)?;
                    Ok(Variant::Integer(
                        m.parent
                            .and_then(|p| {
                                rt.host().menus[&p].children.iter().position(|c| *c == h.0)
                            })
                            .unwrap_or(0) as i64,
                    ))
                },
                |rt, this, v| {
                    let h = this_of(rt, this, "MenuItem.index")?;
                    if let Some(parent) = state(rt, h)?.parent {
                        let max = state(rt, ObjectHandle(parent))?
                            .children
                            .len()
                            .saturating_sub(1);
                        attach(
                            rt,
                            ObjectHandle(parent),
                            h,
                            (v.to_integer()?.max(0) as usize).min(max),
                        )?;
                    }
                    Ok(())
                },
            ),
            (
                "shortcut",
                |_, _| Ok(Variant::String(String::new())),
                |_, _, value| {
                    if value.to_tjs_string()?.is_empty() {
                        Ok(())
                    } else {
                        Err(unsupported(
                            "MenuItem.shortcut",
                            "native accelerator action mapping",
                        ))
                    }
                },
            ),
        ],
    );
}
