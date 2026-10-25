// SPDX-License-Identifier: MPL-2.0
//! Retained widgets/layout/focus/input and rendering. Button capture/release and
//! letterbox mapping are ported from RFVP exit_confirm_ui.rs (see UPSTREAM.md).
use crate::{
    input::{InputState, VmKey, VmMouseButton},
    layer::{intersect, rect_valid, rectangle, Paint, Primitive},
    world::WorldCommand,
};
use anyhow::{ensure, Context, Result};
use crysirror_core::{service::InputFrame, state::ClientId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
fn yes() -> bool {
    true
}
fn color() -> [u8; 4] {
    [52, 66, 88, 255]
}
fn size() -> u16 {
    18
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidgetKind {
    /// Container with a fill.
    #[default]
    Panel,
    /// Text, not focusable.
    Label,
    /// Activates on Enter or a matching pointer release.
    Button,
    /// Button that flips `value` between 0 and 1.
    Toggle,
    /// Horizontal 0..1 value; Left/Right or drag.
    Slider,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Flow {
    /// Children use their own rects.
    #[default]
    Absolute,
    /// Children laid out left to right.
    Row,
    /// Children laid out top to bottom.
    Column,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetSpec {
    /// Client-local widget id.
    pub id: u32,
    /// Parent widget id.
    #[serde(default)]
    pub parent: Option<u32>,
    /// Rectangle [x, y, w, h], relative to the parent content box.
    pub rect: [f32; 4],
    /// Panel, label, button, toggle or slider.
    #[serde(default)]
    pub kind: WidgetKind,
    /// Layout of children: absolute, row or column.
    #[serde(default)]
    pub flow: Flow,
    /// Inner padding (px).
    #[serde(default)]
    pub padding: u16,
    /// Spacing between flowed children (px).
    #[serde(default)]
    pub gap: u16,
    /// Draw order among siblings.
    #[serde(default)]
    pub z: i32,
    /// Inherited visibility.
    #[serde(default = "yes")]
    pub visible: bool,
    /// Accepts focus and input.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Clip children to this widget.
    #[serde(default)]
    pub clip_children: bool,
    /// While visible, the topmost modal subtree receives all input.
    #[serde(default)]
    pub modal: bool,
    /// RGBA8 fill color.
    #[serde(default = "color")]
    pub color: [u8; 4],
    /// Label or button text.
    #[serde(default)]
    pub text: String,
    /// Explicit licensed project font alias.
    #[serde(default)]
    pub font: Option<String>,
    /// Text pixel size.
    #[serde(default = "size")]
    pub text_size: u16,
    /// World commands run when activated (checked typed data, not script).
    #[serde(default)]
    pub on_activate: Vec<WorldCommand>,
    /// Slider value 0..1, or toggle state (on above 0.5).
    #[serde(default)]
    pub value: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Widget {
    spec: WidgetSpec,
    value: f32,
    checked: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetId {
    pub owner: u32,
    pub id: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiEvent {
    pub widget: WidgetId,
    pub kind: String,
    pub value: f32,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiSystem {
    clients: BTreeMap<u32, BTreeMap<u32, Widget>>,
    pub focus: Option<WidgetId>,
    pub events: Vec<UiEvent>,
    #[serde(skip)]
    ignore_held: bool,
    #[serde(skip)]
    input: InputState,
    #[serde(skip)]
    hover: Option<WidgetId>,
    #[serde(skip)]
    pressed: Option<WidgetId>,
}
struct Layout {
    id: WidgetId,
    rect: [f32; 4],
    clip: [f32; 4],
    z: i32,
    enabled: bool,
    ancestry: Vec<u32>,
    modal: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiCommand {
    Put {
        widget: WidgetSpec,
    },
    Remove {
        id: u32,
    },
    SetVisible {
        id: u32,
        visible: bool,
    },
    SetEnabled {
        id: u32,
        enabled: bool,
    },
    SetValue {
        id: u32,
        value: f32,
    },
    SetText {
        id: u32,
        text: String,
        font: Option<String>,
    },
}
impl UiSystem {
    pub fn apply_batch(&mut self, client: ClientId, commands: &[UiCommand]) -> Result<()> {
        ensure!(commands.len() <= 4096, "UI command limit");
        let mut staged = self.clone();
        let owner = staged.clients.entry(client.0).or_default();
        for command in commands {
            match command {
                UiCommand::Put { widget } => {
                    owner.insert(
                        widget.id,
                        Widget {
                            spec: widget.clone(),
                            value: widget.value,
                            checked: widget.value > 0.5,
                        },
                    );
                }
                UiCommand::Remove { id } => {
                    ensure!(owner.contains_key(id), "widget missing");
                    let mut remove = BTreeSet::from([*id]);
                    loop {
                        let n = remove.len();
                        for (id, w) in owner.iter() {
                            if w.spec.parent.is_some_and(|p| remove.contains(&p)) {
                                remove.insert(*id);
                            }
                        }
                        if n == remove.len() {
                            break;
                        }
                    }
                    owner.retain(|id, _| !remove.contains(id));
                }
                other => {
                    let id = match other {
                        UiCommand::SetVisible { id, .. }
                        | UiCommand::SetEnabled { id, .. }
                        | UiCommand::SetValue { id, .. }
                        | UiCommand::SetText { id, .. } => id,
                        _ => unreachable!(),
                    };
                    let w = owner.get_mut(id).context("widget missing")?;
                    match other {
                        UiCommand::SetVisible { visible, .. } => w.spec.visible = *visible,
                        UiCommand::SetEnabled { enabled, .. } => w.spec.enabled = *enabled,
                        UiCommand::SetValue { value, .. } => {
                            w.value = *value;
                            w.checked = *value > 0.5;
                        }
                        UiCommand::SetText { text, font, .. } => {
                            w.spec.text = text.clone();
                            w.spec.font = font.clone();
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
        let exists = |id: WidgetId| {
            staged
                .clients
                .get(&id.owner)
                .is_some_and(|c| c.contains_key(&id.id))
        };
        staged.focus = staged.focus.filter(|id| exists(*id));
        staged.pressed = staged.pressed.filter(|id| exists(*id));
        staged.hover = staged.hover.filter(|id| exists(*id));
        staged.validate()?;
        *self = staged;
        Ok(())
    }
    pub fn reset_input_after_restore(&mut self) {
        self.input = InputState::default();
        self.pressed = None;
        self.hover = None;
        self.ignore_held = true;
    }

    pub fn client_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.clients.keys().copied()
    }
    pub fn install(&mut self, client: ClientId, specs: &[WidgetSpec]) -> Result<()> {
        let mut staged = self.clone();
        let owner = staged.clients.entry(client.0).or_default();
        for spec in specs {
            ensure!(!owner.contains_key(&spec.id), "duplicate widget id");
            owner.insert(
                spec.id,
                Widget {
                    spec: spec.clone(),
                    value: spec.value,
                    checked: spec.value > 0.5,
                },
            );
        }
        staged.validate()?;
        *self = staged;
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.clients.len() <= 64
                && self.clients.values().map(BTreeMap::len).sum::<usize>() <= 4096
                && self.events.len() <= 256,
            "UI size limit"
        );
        for owner in self.clients.values() {
            for (id, widget) in owner {
                let s = &widget.spec;
                ensure!(
                    *id == s.id
                        && rect_valid(s.rect)
                        && s.padding <= 8192
                        && s.gap <= 8192
                        && s.text.len() <= 1024 * 1024
                        && (1..=512).contains(&s.text_size),
                    "invalid widget/bounds/text"
                );
                ensure!(
                    widget.value.is_finite()
                        && (0.0..=1.0).contains(&widget.value)
                        && s.value.is_finite()
                        && (0.0..=1.0).contains(&s.value),
                    "invalid UI value"
                );
                ensure!(
                    s.text.is_empty() || s.font.as_ref().is_some_and(|f| !f.is_empty()),
                    "text widget requires font alias"
                );
                ensure!(s.on_activate.len() <= 128, "UI action count");
                let mut seen = BTreeSet::from([*id]);
                let mut p = s.parent;
                while let Some(id) = p {
                    ensure!(seen.len() <= 64 && seen.insert(id), "UI parent cycle/depth");
                    p = owner.get(&id).context("UI parent missing")?.spec.parent;
                }
            }
        }
        if let Some(f) = self.focus {
            ensure!(
                self.clients
                    .get(&f.owner)
                    .is_some_and(|c| c.contains_key(&f.id)),
                "saved UI focus missing"
            );
        }
        Ok(())
    }
    fn layout(&self, canvas: [u32; 2]) -> Result<Vec<Layout>> {
        self.validate()?;
        ensure!(
            canvas.iter().all(|v| *v > 0 && *v <= 8192),
            "UI canvas bounds"
        );
        fn resolve(
            id: u32,
            owner: &BTreeMap<u32, Widget>,
            cache: &mut BTreeMap<u32, Option<([f32; 4], [f32; 4], i32, bool, Vec<u32>, bool)>>,
            canvas: [u32; 2],
        ) -> Result<Option<([f32; 4], [f32; 4], i32, bool, Vec<u32>, bool)>> {
            if let Some(x) = cache.get(&id) {
                return Ok(x.clone());
            }
            let s = &owner.get(&id).context("UI widget missing")?.spec;
            if !s.visible {
                cache.insert(id, None);
                return Ok(None);
            }
            let (mut rect, mut clip, mut z, mut enabled, mut ancestry, mut modal) = (
                s.rect,
                [0., 0., canvas[0] as f32, canvas[1] as f32],
                s.z,
                s.enabled,
                vec![id],
                s.modal,
            );
            if let Some(p) = s.parent {
                let Some((pr, pc, pz, pe, pa, pm)) = resolve(p, owner, cache, canvas)? else {
                    cache.insert(id, None);
                    return Ok(None);
                };
                let ps = &owner[&p].spec;
                let pad = f32::from(ps.padding);
                rect[0] += pr[0] + pad;
                rect[1] += pr[1] + pad;
                if ps.flow != Flow::Absolute {
                    let offset: f32 = owner
                        .values()
                        .filter(|w| w.spec.parent == Some(p) && w.spec.id < id && w.spec.visible)
                        .map(|w| {
                            if ps.flow == Flow::Row {
                                w.spec.rect[2] + f32::from(ps.gap)
                            } else {
                                w.spec.rect[3] + f32::from(ps.gap)
                            }
                        })
                        .sum();
                    if ps.flow == Flow::Row {
                        rect[0] += offset;
                    } else {
                        rect[1] += offset;
                    }
                }
                let clipped = if ps.clip_children {
                    intersect(pc, pr)
                } else {
                    Some(pc)
                };
                let Some(c) = clipped else {
                    cache.insert(id, None);
                    return Ok(None);
                };
                clip = c;
                z = z.saturating_add(pz);
                enabled &= pe;
                ancestry.extend(pa);
                modal |= pm;
            }
            ensure!(rect_valid(rect), "UI layout overflow");
            let result = Some((rect, clip, z, enabled, ancestry, modal));
            cache.insert(id, result.clone());
            Ok(result)
        }
        let mut out = Vec::new();
        for (client, owner) in &self.clients {
            let mut cache = BTreeMap::new();
            for id in owner.keys() {
                if let Some((rect, clip, z, enabled, ancestry, modal)) =
                    resolve(*id, owner, &mut cache, canvas)?
                {
                    out.push(Layout {
                        id: WidgetId {
                            owner: *client,
                            id: *id,
                        },
                        rect,
                        clip,
                        z,
                        enabled,
                        ancestry,
                        modal,
                    });
                }
            }
        }
        out.sort_by_key(|l| (l.modal, l.z, l.id.owner, l.id.id));
        Ok(out)
    }
    pub fn tick(
        &mut self,
        mut frame: InputFrame,
        canvas: [u32; 2],
    ) -> Result<(Vec<(u32, Vec<WorldCommand>)>, bool)> {
        if self.ignore_held {
            if frame.buttons_down == 0 {
                self.ignore_held = false;
            } else {
                frame.buttons_down = 0;
                frame.buttons_pressed = 0;
            }
        }
        let layouts = self.layout(canvas)?;
        self.input.next_frame();
        self.input.on_mouse_move(frame.pointer_x, frame.pointer_y);
        let mouse = 1 << 16;
        let down = frame.buttons_down & mouse != 0;
        if (frame.buttons_pressed & mouse != 0)
            || (down && !self.input.is_mouse_down(VmMouseButton::Left))
        {
            self.input.on_mouse_down(VmMouseButton::Left);
        }
        if !down && self.input.is_mouse_down(VmMouseButton::Left) {
            self.input.on_mouse_up(VmMouseButton::Left);
        }
        for (bit, key) in [
            (1, VmKey::Enter),
            (2, VmKey::Space),
            (4, VmKey::ArrowLeft),
            (8, VmKey::ArrowRight),
            (16, VmKey::ArrowUp),
            (32, VmKey::ArrowDown),
            (64, VmKey::Tab),
        ] {
            if frame.buttons_pressed & bit != 0
                || (frame.buttons_down & bit != 0 && !self.input.is_key_down(key))
            {
                self.input.on_key_down(key);
            }
            if frame.buttons_down & bit == 0 && self.input.is_key_down(key) {
                self.input.on_key_up(key);
            }
        }
        let modal = layouts
            .iter()
            .rev()
            .find(|l| self.clients[&l.id.owner][&l.id.id].spec.modal)
            .map(|l| l.id);
        let allowed =
            |l: &Layout| modal.is_none_or(|m| m.owner == l.id.owner && l.ancestry.contains(&m.id));
        let interactive = |l: &Layout| {
            l.enabled
                && matches!(
                    self.clients[&l.id.owner][&l.id.id].spec.kind,
                    WidgetKind::Button | WidgetKind::Toggle | WidgetKind::Slider
                )
                && allowed(l)
        };
        let focusable = layouts
            .iter()
            .filter(|l| interactive(l))
            .map(|l| l.id)
            .collect::<Vec<_>>();
        if self.focus.is_none_or(|f| !focusable.contains(&f)) {
            self.focus = focusable.first().copied();
        }
        if self.input.vk_down_stock(0x09) || self.input.vk_down_stock(0x28) {
            if !focusable.is_empty() {
                let next = self
                    .focus
                    .and_then(|f| focusable.iter().position(|x| *x == f))
                    .map(|n| (n + 1) % focusable.len())
                    .unwrap_or(0);
                self.focus = Some(focusable[next]);
            }
        }
        self.hover = layouts
            .iter()
            .rev()
            .find(|l| {
                interactive(l)
                    && hit(frame.pointer_x, frame.pointer_y, l.rect)
                    && hit(frame.pointer_x, frame.pointer_y, l.clip)
            })
            .map(|l| l.id);
        let hit_ui = layouts.iter().rev().any(|l| {
            allowed(l)
                && hit(frame.pointer_x, frame.pointer_y, l.rect)
                && hit(frame.pointer_x, frame.pointer_y, l.clip)
        });
        let mut activate = None;
        if self.input.vk_down_stock(0x01) {
            self.pressed = self.hover;
            if self.hover.is_some() {
                self.focus = self.hover;
            }
        }
        if self.input.vk_up_stock(0x01) {
            let pressed = self.pressed.take();
            if pressed.is_some() && pressed == self.hover {
                activate = pressed;
            }
        }
        if self.input.vk_down_stock(0x0d) || self.input.vk_down_stock(0x20) {
            activate = self.focus;
        }
        let mut actions = Vec::new();
        if let Some(id) = self.pressed.or(self.focus) {
            if let Some(l) = layouts.iter().find(|l| l.id == id) {
                if self.clients[&id.owner][&id.id].spec.kind == WidgetKind::Slider
                    && ((self.pressed == Some(id) && down)
                        || self.input.vk_down_stock(0x25)
                        || self.input.vk_down_stock(0x27))
                {
                    let w = self
                        .clients
                        .get_mut(&id.owner)
                        .unwrap()
                        .get_mut(&id.id)
                        .unwrap();
                    w.value = if self.pressed == Some(id) && down {
                        ((frame.pointer_x as f32 - l.rect[0]) / l.rect[2]).clamp(0., 1.)
                    } else {
                        (w.value
                            + if self.input.vk_down_stock(0x27) {
                                0.05
                            } else {
                                -0.05
                            })
                        .clamp(0., 1.)
                    };
                    activate = Some(id);
                }
            }
        }
        if let Some(id) = activate {
            let w = self
                .clients
                .get_mut(&id.owner)
                .unwrap()
                .get_mut(&id.id)
                .unwrap();
            if w.spec.kind == WidgetKind::Toggle {
                w.checked = !w.checked;
                w.value = if w.checked { 1. } else { 0. };
            }
            actions.push((id.owner, w.spec.on_activate.clone()));
            if self.events.len() == 256 {
                self.events.remove(0);
            }
            self.events.push(UiEvent {
                widget: id,
                kind: if w.spec.kind == WidgetKind::Button {
                    "activate"
                } else {
                    "change"
                }
                .into(),
                value: w.value,
            });
        }
        let keyboard = (frame.buttons_pressed & 127 != 0) && self.focus.is_some();
        Ok((
            actions,
            modal.is_some() || hit_ui || self.pressed.is_some() || keyboard,
        ))
    }
    pub fn frame(&self, canvas: [u32; 2]) -> Result<Vec<Primitive>> {
        let mut out = Vec::new();
        for l in self.layout(canvas)? {
            let w = &self.clients[&l.id.owner][&l.id.id];
            let s = &w.spec;
            let order = [2_000_000_000 + i32::from(l.modal), l.z];
            let mut color = s.color;
            if !l.enabled {
                for c in &mut color[..3] {
                    *c /= 2;
                }
            } else if self.pressed == Some(l.id) {
                for c in &mut color[..3] {
                    *c = c.saturating_sub(25);
                }
            } else if self.hover == Some(l.id) || self.focus == Some(l.id) {
                for c in &mut color[..3] {
                    *c = c.saturating_add(24);
                }
            }
            if s.kind != WidgetKind::Label {
                out.push(rectangle(
                    l.id.owner,
                    l.id.id,
                    0,
                    l.rect,
                    Paint::Solid { color },
                    Some(l.clip),
                    order,
                ));
            }
            if s.kind == WidgetKind::Toggle && w.checked {
                let mut r = l.rect;
                r[0] += 5.;
                r[1] += 5.;
                r[2] = (r[2] - 10.).max(1.);
                r[3] = (r[3] - 10.).max(1.);
                out.push(rectangle(
                    l.id.owner,
                    l.id.id,
                    1,
                    r,
                    Paint::Solid {
                        color: [64, 210, 120, 255],
                    },
                    Some(l.clip),
                    order,
                ));
            }
            if s.kind == WidgetKind::Slider {
                let mut r = l.rect;
                r[2] = (r[2] * w.value).max(1.);
                out.push(rectangle(
                    l.id.owner,
                    l.id.id,
                    1,
                    r,
                    Paint::Solid {
                        color: [70, 170, 225, 255],
                    },
                    Some(l.clip),
                    order,
                ));
            }
            if !s.text.is_empty() {
                let mut r = l.rect;
                r[0] += 6.;
                r[1] += 4.;
                r[2] = (r[2] - 12.).max(1.);
                r[3] = (r[3] - 8.).max(1.);
                let text = s.text.replace("{value}", &format!("{:.2}", w.value));
                out.push(rectangle(
                    l.id.owner,
                    l.id.id,
                    2,
                    r,
                    Paint::Text {
                        text,
                        font: s.font.clone().context("UI font missing")?,
                        size: s.text_size,
                        color: [245, 245, 245, 255],
                        layout: crysirror_core::service::SIMPLE_TEXT_LAYOUT.into(),
                    },
                    Some(l.clip),
                    order,
                ));
            }
        }
        Ok(out)
    }
}
fn hit(x: i32, y: i32, r: [f32; 4]) -> bool {
    inside_rect(
        x,
        y,
        (
            r[0].floor() as i32,
            r[1].floor() as i32,
            r[2].ceil() as i32,
            r[3].ceil() as i32,
        ),
    )
}

// The following coordinate mapping implementation is ported from RFVP.
pub struct UiSurfaceTransform {
    scale: f32,
    off_x: f32,
    off_y: f32,
    content_x: i32,
    content_y: i32,
    content_w: i32,
    content_h: i32,
}

impl UiSurfaceTransform {
    pub fn content_rect(&self) -> (i32, i32, i32, i32) {
        (
            self.content_x,
            self.content_y,
            self.content_w,
            self.content_h,
        )
    }
    pub fn new(virtual_size: (u32, u32), surface_size: (u32, u32)) -> Self {
        let vw = virtual_size.0.max(1) as f32;
        let vh = virtual_size.1.max(1) as f32;
        let sw = surface_size.0.max(1) as f32;
        let sh = surface_size.1.max(1) as f32;
        let scale = (sw / vw).min(sh / vh);
        let dst_w = vw * scale;
        let dst_h = vh * scale;
        let off_x = (sw - dst_w) * 0.5;
        let off_y = (sh - dst_h) * 0.5;
        Self {
            scale,
            off_x,
            off_y,
            content_x: off_x.round() as i32,
            content_y: off_y.round() as i32,
            content_w: dst_w.round() as i32,
            content_h: dst_h.round() as i32,
        }
    }
    pub fn map_x(&self, x: i32) -> i32 {
        (self.off_x + x as f32 * self.scale).round() as i32
    }
    pub fn map_y(&self, y: i32) -> i32 {
        (self.off_y + y as f32 * self.scale).round() as i32
    }
    pub fn map_w(&self, w: i32) -> i32 {
        ((w as f32 * self.scale).round() as i32).max(if w > 0 { 1 } else { 0 })
    }
    pub fn map_h(&self, h: i32) -> i32 {
        ((h as f32 * self.scale).round() as i32).max(if h > 0 { 1 } else { 0 })
    }
    pub fn map_rect(&self, x: i32, y: i32, w: i32, h: i32) -> (i32, i32, i32, i32) {
        (self.map_x(x), self.map_y(y), self.map_w(w), self.map_h(h))
    }
    pub fn scale_font(&self, size: f32) -> f32 {
        (size * self.scale).max(1.0)
    }
}

fn inside_rect(x: i32, y: i32, rect: (i32, i32, i32, i32)) -> bool {
    let (rx, ry, rw, rh) = rect;
    x >= rx && x < rx + rw && y >= ry && y < ry + rh
}

pub fn map_window_to_virtual(
    px: f64,
    py: f64,
    surface_size: (u32, u32),
    virtual_size: (u32, u32),
) -> Option<(i32, i32)> {
    let (sw_u, sh_u) = surface_size;
    let (vw_u, vh_u) = virtual_size;
    let sw = sw_u.max(1) as f64;
    let sh = sh_u.max(1) as f64;
    let vw = vw_u.max(1) as f64;
    let vh = vh_u.max(1) as f64;
    let scale = (sw / vw).min(sh / vh);
    let dst_w = vw * scale;
    let dst_h = vh * scale;
    let off_x = (sw - dst_w) * 0.5;
    let off_y = (sh - dst_h) * 0.5;
    if px < off_x || px >= off_x + dst_w || py < off_y || py >= off_y + dst_h {
        return None;
    }
    let vx = ((px - off_x) / scale) as i32;
    let vy = ((py - off_y) / scale) as i32;
    Some((vx.clamp(0, vw as i32 - 1), vy.clamp(0, vh as i32 - 1)))
}
