// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Engine-owned menu trees, popup layout, radio state and ordered activation.
use super::{UiEvent, WidgetId};
use crate::layer::{Paint, Primitive, rect_valid, rectangle};
use anyhow::{Context, Result, ensure};
use crysirror_core::service::{InputFrame, buttons};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
fn yes() -> bool {
    true
}
fn row() -> f32 {
    24.
}
fn cell() -> f32 {
    100.
}
fn popup() -> f32 {
    240.
}
fn size() -> u16 {
    18
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// One logical menu item; children and radio groups are engine-owned.
pub struct MenuItemSpec {
    /// Item ID, unique within this menu tree.
    pub id: u32,
    /// Display text without native platform mnemonics.
    pub label: String,
    /// Whether this item participates in layout.
    #[serde(default = "yes")]
    pub visible: bool,
    /// Whether input can activate or open this item.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Displayed check/radio state.
    #[serde(default)]
    pub checked: bool,
    /// Selecting this item clears radio siblings in the same group.
    #[serde(default)]
    pub radio: bool,
    /// Sibling radio group identity.
    #[serde(default)]
    pub group: u32,
    /// Render a noninteractive separator row.
    #[serde(default)]
    pub separator: bool,
    /// Nested menu items, at most sixteen levels.
    #[serde(default)]
    pub children: Vec<MenuItemSpec>,
}
impl MenuItemSpec {
    /// Same sibling radio-group rule for data users and native property adapters.
    pub fn set_checked(items: &mut [Self], id: u32, checked: bool) -> bool {
        if let Some(index) = items.iter().position(|item| item.id == id) {
            let (radio, group) = (items[index].radio, items[index].group);
            if checked && radio {
                for item in items.iter_mut() {
                    if item.radio && item.group == group {
                        item.checked = false;
                    }
                }
            }
            items[index].checked = checked;
            return true;
        }
        items
            .iter_mut()
            .any(|item| Self::set_checked(&mut item.children, id, checked))
    }
    pub fn find(items: &[Self], id: u32) -> Option<&Self> {
        for item in items {
            if item.id == id {
                return Some(item);
            }
            if let Some(item) = Self::find(&item.children, id) {
                return Some(item);
            }
        }
        None
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// A retained menu bar or context menu, scoped to its owning client.
pub struct MenuSpec {
    /// Menu ID, unique within the owning client (1..65535).
    pub id: u32,

    /// Logical canvas rectangle anchoring the menu bar.
    pub rect: [f32; 4],

    /// Shared engine font alias.
    pub font: String,
    /// Nominal font size in logical pixels.
    #[serde(default = "size")]
    pub text_size: u16,
    /// Width of a horizontal bar item.
    #[serde(default = "cell")]
    pub cell_width: f32,
    /// Width of a vertical submenu.
    #[serde(default = "popup")]
    pub popup_width: f32,
    /// Height of each bar item or submenu row.
    #[serde(default = "row")]
    pub row_height: f32,
    /// Whether this menu can be drawn or receive input.
    #[serde(default = "yes")]
    pub visible: bool,
    /// A context-only tree is hidden until open_menu is requested.
    #[serde(default)]
    pub context: bool,

    /// Root menu items.
    pub items: Vec<MenuItemSpec>,
}
impl MenuSpec {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.id > 0
                && self.id <= 65535
                && rect_valid(self.rect)
                && !self.font.is_empty()
                && self.font.len() <= 128
                && (1..=512).contains(&self.text_size),
            "menu geometry/font/id"
        );
        ensure!(
            [self.cell_width, self.popup_width, self.row_height]
                .iter()
                .all(|n| n.is_finite() && *n >= 8. && *n <= 8192.),
            "menu row/popup limits"
        );
        fn check(items: &[MenuItemSpec], depth: u8, ids: &mut BTreeSet<u32>) -> Result<()> {
            ensure!(depth <= 16, "menu nesting limit");
            let mut checked = BTreeSet::new();
            for item in items {
                ensure!(
                    item.id > 0
                        && item.id <= 65535
                        && ids.insert(item.id)
                        && ids.len() <= 1024
                        && item.label.len() <= 4096,
                    "menu item id/count/label limit"
                );
                if item.radio && item.checked {
                    ensure!(
                        checked.insert(item.group),
                        "multiple checked items in one radio group"
                    );
                }
                check(&item.children, depth + 1, ids)?;
            }
            Ok(())
        }
        check(&self.items, 0, &mut BTreeSet::new())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Menu {
    spec: MenuSpec,
    #[serde(default)]
    open: Vec<u32>,
    #[serde(default)]
    position: Option<[f32; 2]>,
}
impl Menu {
    fn valid_path(spec: &MenuSpec, path: &[u32]) -> Vec<u32> {
        let mut items = spec.items.as_slice();
        let mut valid = Vec::new();
        for id in path.iter().take(16) {
            let Some(item) = items.iter().find(|item| {
                item.id == *id && item.visible && item.enabled && !item.children.is_empty()
            }) else {
                break;
            };
            valid.push(*id);
            items = &item.children;
        }
        valid
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MenuSystem {
    clients: BTreeMap<u32, BTreeMap<u32, Menu>>,
    #[serde(skip)]
    pressed: Option<(u32, u32, u32)>,
    #[serde(skip)]
    focus: Option<(u32, u32, u32)>,
    #[serde(skip)]
    down: bool,
}
struct Cell {
    owner: u32,
    bar: u32,
    item: u32,
    rect: [f32; 4],
    path: Vec<u32>,
    enabled: bool,
    label: String,
    checked: bool,
    children: bool,
    separator: bool,
}
impl MenuSystem {
    pub fn is_empty(&self) -> bool {
        self.clients.values().all(BTreeMap::is_empty)
    }
    pub fn client_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.clients.keys().copied()
    }
    pub fn put(&mut self, owner: u32, spec: MenuSpec) -> Result<()> {
        spec.validate()?;
        let bars = self.clients.entry(owner).or_default();
        ensure!(
            bars.contains_key(&spec.id) || bars.len() < 16,
            "menu bar limit"
        );
        let (open, position) = bars
            .get(&spec.id)
            .map(|m| (m.open.clone(), m.position))
            .unwrap_or_default();
        let open = Menu::valid_path(&spec, &open);
        let position = position.filter(|_| spec.context && spec.visible);
        bars.insert(
            spec.id,
            Menu {
                spec,
                open,
                position,
            },
        );
        Ok(())
    }
    pub fn remove(&mut self, owner: u32, id: u32) -> Result<()> {
        self.clients
            .get_mut(&owner)
            .and_then(|bars| bars.remove(&id))
            .context("menu missing")?;
        Ok(())
    }
    pub fn open(
        &mut self,
        owner: u32,
        bar: u32,
        item: Option<u32>,
        position: Option<[f32; 2]>,
    ) -> Result<()> {
        let menu = self
            .clients
            .get_mut(&owner)
            .and_then(|bars| bars.get_mut(&bar))
            .context("menu missing")?;
        ensure!(
            position.is_none_or(|p| p.iter().all(|v| v.is_finite())),
            "menu position"
        );
        if let Some(id) = item {
            ensure!(
                menu.spec.items.iter().any(|item| item.id == id
                    && item.visible
                    && item.enabled
                    && !item.children.is_empty()),
                "menu root is not openable"
            );
            menu.open = vec![id];
        }
        if menu.spec.context {
            menu.position = Some(position.unwrap_or([menu.spec.rect[0], menu.spec.rect[1]]));
        }
        Ok(())
    }
    pub fn close(&mut self, owner: u32, bar: u32) -> Result<()> {
        let menu = self
            .clients
            .get_mut(&owner)
            .and_then(|bars| bars.get_mut(&bar))
            .context("menu missing")?;
        menu.open.clear();
        menu.position = None;
        Ok(())
    }
    pub fn checked(&mut self, owner: u32, bar: u32, item: u32, checked: bool) -> Result<()> {
        let menu = self
            .clients
            .get_mut(&owner)
            .and_then(|bars| bars.get_mut(&bar))
            .context("menu missing")?;
        ensure!(
            MenuItemSpec::set_checked(&mut menu.spec.items, item, checked),
            "menu item missing"
        );
        Ok(())
    }
    pub fn get(&self, owner: u32, bar: u32) -> Option<&MenuSpec> {
        self.clients.get(&owner)?.get(&bar).map(|m| &m.spec)
    }
    pub fn validate(&self) -> Result<()> {
        for bars in self.clients.values() {
            ensure!(bars.len() <= 16, "menu bar limit");
            for (id, m) in bars {
                ensure!(*id == m.spec.id, "menu identity mismatch");
                m.spec.validate()?;
                ensure!(
                    Menu::valid_path(&m.spec, &m.open) == m.open,
                    "invalid saved menu branch"
                );
                ensure!(
                    m.position
                        .is_none_or(|p| m.spec.context && p.iter().all(|v| v.is_finite())),
                    "invalid saved menu position"
                );
            }
        }
        Ok(())
    }
    pub fn reset_input(&mut self) {
        self.pressed = None;
        self.focus = None;
        self.down = false;
        for bars in self.clients.values_mut() {
            for menu in bars.values_mut() {
                menu.open.clear();
                menu.position = None;
            }
        }
    }
    fn layout(&self, canvas: [u32; 2]) -> Vec<Cell> {
        let mut cells = vec![];
        for (owner, bars) in &self.clients {
            for (bar, menu) in bars {
                if !menu.spec.visible || menu.spec.context && menu.position.is_none() {
                    continue;
                }
                let s = &menu.spec;
                let mut items = s.items.as_slice();
                let mut path = vec![];
                let mut x = s.rect[0];
                let mut y = s.rect[1];
                let mut horizontal = !s.context;
                if let Some(pos) = menu.position {
                    x = pos[0];
                    y = pos[1];
                }
                for depth in 0..=16 {
                    let width = if horizontal {
                        s.cell_width
                    } else {
                        s.popup_width
                    }
                    .min(canvas[0] as f32);
                    let height = s.row_height;
                    if !horizontal {
                        x = x.clamp(0., (canvas[0] as f32 - width).max(0.));
                        y = y.clamp(
                            0.,
                            (canvas[1] as f32
                                - height * items.iter().filter(|i| i.visible).count() as f32)
                                .max(0.),
                        );
                    }
                    let mut selected = None;
                    let mut at = 0f32;
                    for item in items.iter().filter(|i| i.visible) {
                        let rect = if horizontal {
                            [x + at, y, width, height]
                        } else {
                            [x, y + at, width, height]
                        };
                        let mut branch = path.clone();
                        branch.push(item.id);
                        cells.push(Cell {
                            owner: *owner,
                            bar: *bar,
                            item: item.id,
                            rect,
                            path: branch.clone(),
                            enabled: item.enabled,
                            label: item.label.clone(),
                            checked: item.checked,
                            children: !item.children.is_empty(),
                            separator: item.separator,
                        });
                        if menu.open.get(depth) == Some(&item.id) && item.enabled {
                            selected = Some((item, rect, branch));
                        }
                        at += if horizontal { width } else { height };
                    }
                    let Some((item, rect, branch)) = selected else {
                        break;
                    };
                    if item.children.is_empty() {
                        break;
                    }
                    items = &item.children;
                    path = branch;
                    if horizontal {
                        x = rect[0];
                        y = rect[1] + height;
                    } else {
                        x = rect[0] + width;
                        y = rect[1];
                    }
                    horizontal = false;
                }
            }
        }
        cells
    }
    fn activate(&mut self, cell: &Cell) -> Result<Option<UiEvent>> {
        if !cell.enabled || cell.separator {
            return Ok(None);
        }
        let menu = self
            .clients
            .get_mut(&cell.owner)
            .unwrap()
            .get_mut(&cell.bar)
            .unwrap();
        if cell.children {
            if menu.open == cell.path {
                menu.open.pop();
            } else {
                menu.open = cell.path.clone();
            }
            return Ok(None);
        }
        let item = MenuItemSpec::find(&menu.spec.items, cell.item).unwrap();
        if item.radio {
            MenuItemSpec::set_checked(&mut menu.spec.items, cell.item, true);
        }
        let checked = MenuItemSpec::find(&menu.spec.items, cell.item)
            .unwrap()
            .checked;
        menu.open.clear();
        menu.position = None;
        Ok(Some(UiEvent {
            widget: WidgetId {
                owner: cell.owner,
                id: cell.item,
            },
            kind: "menu_activate".into(),
            value: if checked { 1. } else { 0. },
            menu_bar: Some(cell.bar),
        }))
    }
    pub fn tick(&mut self, frame: InputFrame, canvas: [u32; 2]) -> Result<(Vec<UiEvent>, bool)> {
        let cells = self.layout(canvas);
        let point = [frame.pointer_x as f32, frame.pointer_y as f32];
        let hit = cells.iter().rev().find(|c| {
            point[0] >= c.rect[0]
                && point[1] >= c.rect[1]
                && point[0] < c.rect[0] + c.rect[2]
                && point[1] < c.rect[1] + c.rect[3]
        });
        let modal = self.clients.values().any(|bars| {
            bars.values()
                .any(|m| m.spec.visible && (!m.open.is_empty() || m.position.is_some()))
        });
        let down = frame.buttons_down & buttons::MOUSE_LEFT != 0;
        let press = frame.buttons_pressed & buttons::MOUSE_LEFT != 0 || down && !self.down;
        let release = !down && (self.down || press);
        self.down = down;
        let mut consumed = modal;
        let mut events = vec![];
        if frame.buttons_pressed & buttons::ESCAPE != 0 && modal {
            self.reset_input();
            self.down = down;
            return Ok((events, true));
        }
        if press {
            if let Some(hit) = hit {
                self.pressed = Some((hit.owner, hit.bar, hit.item));
                self.focus = self.pressed;
                consumed = true;
            } else if modal {
                self.reset_input();
                self.down = down;
                return Ok((events, true));
            }
        }
        if release {
            if let Some(pressed) = self.pressed.take() {
                consumed = true;
                if let Some(hit) = hit.filter(|hit| pressed == (hit.owner, hit.bar, hit.item)) {
                    if let Some(event) = self.activate(hit)? {
                        events.push(event);
                    }
                }
            }
        }
        if modal && frame.buttons_pressed & (buttons::UP | buttons::DOWN) != 0 {
            let deepest = cells
                .iter()
                .filter(|c| c.enabled && !c.separator)
                .map(|c| c.path.len())
                .max()
                .unwrap_or(0);
            let choices = cells
                .iter()
                .filter(|c| c.enabled && !c.separator && c.path.len() == deepest)
                .collect::<Vec<_>>();
            if !choices.is_empty() {
                let current = choices
                    .iter()
                    .position(|c| self.focus == Some((c.owner, c.bar, c.item)))
                    .unwrap_or(0);
                let next = if frame.buttons_pressed & buttons::DOWN != 0 {
                    (current + 1) % choices.len()
                } else {
                    (current + choices.len() - 1) % choices.len()
                };
                let c = choices[next];
                self.focus = Some((c.owner, c.bar, c.item));
            }
        }
        if modal && frame.buttons_pressed & buttons::ENTER != 0 {
            if let Some(c) = cells
                .iter()
                .find(|c| self.focus == Some((c.owner, c.bar, c.item)))
            {
                if let Some(event) = self.activate(c)? {
                    events.push(event);
                }
            }
        }
        Ok((events, consumed))
    }
    pub fn frame(&self, canvas: [u32; 2]) -> Result<Vec<Primitive>> {
        let mut out = vec![];
        let clip = Some([0., 0., canvas[0] as f32, canvas[1] as f32]);
        for (order, c) in self.layout(canvas).into_iter().enumerate() {
            let spec = &self.clients[&c.owner][&c.bar].spec;
            let id = (c.bar << 16) | c.item;
            let order = [i32::MAX - 1, order as i32];
            out.push(rectangle(
                c.owner,
                id,
                200,
                c.rect,
                Paint::Solid {
                    color: if self.focus == Some((c.owner, c.bar, c.item)) {
                        [65, 85, 120, 255]
                    } else {
                        [36, 40, 48, 255]
                    },
                },
                clip,
                order,
            ));
            if c.separator {
                let mut r = c.rect;
                r[1] += r[3] / 2.;
                r[2] -= 4.;
                r[3] = 1.;
                out.push(rectangle(
                    c.owner,
                    id,
                    201,
                    r,
                    Paint::Solid {
                        color: [100, 100, 100, 255],
                    },
                    clip,
                    order,
                ));
                continue;
            }
            let mut rect = c.rect;
            rect[0] += 6.;
            rect[1] += 2.;
            rect[2] = (rect[2] - 12.).max(1.);
            rect[3] = (rect[3] - 4.).max(1.);
            let label = format!(
                "{}{}{}",
                if c.checked { "✓ " } else { "" },
                c.label,
                if c.children && c.path.len() > 1 {
                    "  ›"
                } else {
                    ""
                }
            );
            out.push(rectangle(
                c.owner,
                id,
                202,
                rect,
                Paint::Text {
                    text: label,
                    font: spec.font.clone(),
                    size: spec.text_size,
                    color: if c.enabled {
                        [245, 245, 245, 255]
                    } else {
                        [128, 128, 128, 255]
                    },
                    layout: crysirror_core::service::UNWRAPPED_TEXT_LAYOUT.into(),
                },
                Some(c.rect),
                order,
            ));
        }
        Ok(out)
    }
}
