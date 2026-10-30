// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Engine-owned modal message dialogs: a caption, a wrapped message and a row
//! of buttons, drawn and hit-tested by the engine on every platform. While a
//! dialog is open it takes all UI input; choosing a button queues one
//! `dialog_result` event (value = button id) for the owning client.
use super::{UiEvent, WidgetId};
use crate::layer::{rectangle, Paint, Primitive};
use anyhow::{ensure, Context, Result};
use crysirror_core::service::{buttons, InputFrame, SIMPLE_TEXT_LAYOUT, UNWRAPPED_TEXT_LAYOUT};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
fn size() -> u16 {
    24
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DialogButton {
    /// Value reported in the `dialog_result` event.
    pub id: u32,
    pub label: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DialogSpec {
    /// Dialog id, unique within the owning client (1..65535).
    pub id: u32,
    #[serde(default)]
    pub title: String,
    pub message: String,
    /// Buttons left to right, 1..=8.
    pub buttons: Vec<DialogButton>,
    /// Button focused when the dialog opens (Enter chooses it).
    pub default: u32,
    /// Button chosen by Escape; `None` ignores Escape.
    #[serde(default)]
    pub cancel: Option<u32>,
    /// Shared engine font alias.
    pub font: String,
    #[serde(default = "size")]
    pub text_size: u16,
}
impl DialogSpec {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=65535).contains(&self.id)
                && !self.font.is_empty()
                && self.font.len() <= 128
                && (8..=128).contains(&self.text_size)
                && self.title.len() <= 1024
                && self.message.len() <= 16384,
            "dialog id/font/text limits"
        );
        ensure!((1..=8).contains(&self.buttons.len()), "dialog button count");
        let mut ids = BTreeSet::new();
        for b in &self.buttons {
            ensure!(
                ids.insert(b.id) && !b.label.is_empty() && b.label.len() <= 256,
                "dialog button id/label"
            );
        }
        ensure!(ids.contains(&self.default), "dialog default button missing");
        ensure!(
            self.cancel.is_none_or(|c| ids.contains(&c)),
            "dialog cancel button missing"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Dialog {
    spec: DialogSpec,
    focus: usize,
}
/// Geometry of an open dialog on a canvas.
struct Layout {
    backdrop: [f32; 4],
    panel: [f32; 4],
    title: [f32; 4],
    message: [f32; 4],
    buttons: Vec<[f32; 4]>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DialogSystem {
    /// One open dialog per client; the lowest client id is the active one.
    clients: BTreeMap<u32, Dialog>,
    #[serde(skip)]
    pressed: Option<(u32, usize)>,
}
impl DialogSystem {
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }
    pub fn client_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.clients.keys().copied()
    }
    pub fn get(&self, owner: u32) -> Option<&DialogSpec> {
        self.clients.get(&owner).map(|d| &d.spec)
    }
    pub fn open(&mut self, owner: u32, spec: DialogSpec) -> Result<()> {
        spec.validate()?;
        ensure!(
            !self.clients.contains_key(&owner),
            "a dialog is already open for this client"
        );
        let focus = spec
            .buttons
            .iter()
            .position(|b| b.id == spec.default)
            .unwrap_or(0);
        self.clients.insert(owner, Dialog { spec, focus });
        Ok(())
    }
    pub fn close(&mut self, owner: u32, id: u32) -> Result<()> {
        ensure!(
            self.clients.get(&owner).is_some_and(|d| d.spec.id == id),
            "dialog missing"
        );
        self.clients.remove(&owner);
        self.pressed = None;
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        for d in self.clients.values() {
            d.spec.validate()?;
            ensure!(d.focus < d.spec.buttons.len(), "dialog focus");
        }
        Ok(())
    }
    pub fn reset_input(&mut self) {
        self.pressed = None;
    }
    fn active(&self) -> Option<(u32, &Dialog)> {
        self.clients.iter().next().map(|(o, d)| (*o, d))
    }
    fn layout(d: &Dialog, canvas: [u32; 2]) -> Layout {
        let [cw, ch] = [canvas[0] as f32, canvas[1] as f32];
        let s = f32::from(d.spec.text_size);
        let width = (cw * 0.5).clamp((s * 20.).min(cw - 16.), (cw - 16.).max(1.));
        // A wrapped-line estimate: CJK glyphs are about one em wide.
        let per_line = ((width - 2. * s) / s).max(1.);
        let lines: f32 = d
            .spec
            .message
            .split('\n')
            .map(|l| (l.chars().count() as f32 / per_line).ceil().max(1.))
            .sum();
        let title_h = if d.spec.title.is_empty() { 0. } else { s * 1.6 };
        let message_h = lines * s * 1.4;
        let button_h = s * 2.;
        let height = (s + title_h + message_h + s + button_h + s)
            .min(ch - 16.)
            .max(1.);
        let panel = [(cw - width) / 2., (ch - height) / 2., width, height];
        let title = [
            panel[0] + s,
            panel[1] + s * 0.6,
            width - 2. * s,
            title_h.max(1.),
        ];
        let message = [
            panel[0] + s,
            panel[1] + s + title_h,
            width - 2. * s,
            message_h.max(1.),
        ];
        let n = d.spec.buttons.len() as f32;
        let gap = s * 0.6;
        let bw = ((width - 2. * s - gap * (n - 1.)) / n).min(s * 7.).max(1.);
        let total = bw * n + gap * (n - 1.);
        let by = panel[1] + height - s - button_h;
        let bx = panel[0] + (width - total) / 2.;
        let buttons = (0..d.spec.buttons.len())
            .map(|i| [bx + i as f32 * (bw + gap), by, bw, button_h])
            .collect();
        Layout {
            backdrop: [0., 0., cw, ch],
            panel,
            title,
            message,
            buttons,
        }
    }
    /// Modal input: returns queued events and whether input was consumed
    /// (always, while a dialog is open).
    pub fn tick(&mut self, frame: InputFrame, canvas: [u32; 2]) -> Result<(Vec<UiEvent>, bool)> {
        let Some((owner, d)) = self.active() else {
            return Ok((vec![], false));
        };
        let layout = Self::layout(d, canvas);
        let point = [frame.pointer_x as f32, frame.pointer_y as f32];
        let hit = layout.buttons.iter().position(|r| {
            point[0] >= r[0] && point[1] >= r[1] && point[0] < r[0] + r[2] && point[1] < r[1] + r[3]
        });
        let n = d.spec.buttons.len();
        let mut chosen = None;
        let p = frame.buttons_pressed;
        if p & buttons::MOUSE_LEFT != 0 {
            self.pressed = hit.map(|i| (owner, i));
        }
        if frame.buttons_down & buttons::MOUSE_LEFT == 0 {
            if let Some((o, i)) = self.pressed.take() {
                if o == owner && hit == Some(i) {
                    chosen = Some(i);
                }
            }
        }
        let d = self.clients.get_mut(&owner).context("dialog")?;
        if let Some(i) = hit {
            d.focus = i;
        }
        if p & (buttons::LEFT | buttons::UP) != 0 {
            d.focus = (d.focus + n - 1) % n;
        }
        if p & (buttons::RIGHT | buttons::DOWN | buttons::TAB) != 0 {
            d.focus = (d.focus + 1) % n;
        }
        if p & (buttons::ENTER | buttons::SPACE) != 0 {
            chosen = Some(d.focus);
        }
        if p & buttons::ESCAPE != 0 {
            if let Some(c) = d.spec.cancel {
                chosen = d.spec.buttons.iter().position(|b| b.id == c);
            }
        }
        let mut events = vec![];
        if let Some(i) = chosen {
            let d = self.clients.remove(&owner).context("dialog")?;
            self.pressed = None;
            events.push(UiEvent {
                widget: WidgetId {
                    owner,
                    id: d.spec.id,
                },
                kind: "dialog_result".into(),
                value: d.spec.buttons[i].id as f32,
                menu_bar: None,
            });
        }
        Ok((events, true))
    }
    pub fn frame(&self, canvas: [u32; 2]) -> Result<Vec<Primitive>> {
        let Some((owner, d)) = self.active() else {
            return Ok(vec![]);
        };
        let l = Self::layout(d, canvas);
        let clip = Some(l.backdrop);
        let order = |i: i32| [i32::MAX, i];
        let s = d.spec.text_size;
        // Primitive identity apart from widgets and menus (`bar << 16 | item`).
        let pid = 0xFFFF_0000 | d.spec.id;
        let mut out = vec![
            rectangle(
                owner,
                pid,
                210,
                l.backdrop,
                Paint::Solid {
                    color: [0, 0, 0, 128],
                },
                clip,
                order(0),
            ),
            rectangle(
                owner,
                pid,
                211,
                l.panel,
                Paint::Solid {
                    color: [36, 40, 48, 255],
                },
                clip,
                order(1),
            ),
        ];
        let text = |text: &str, size: u16, color, layout: &str| Paint::Text {
            text: text.to_owned(),
            font: d.spec.font.clone(),
            size,
            color,
            layout: layout.into(),
        };
        if !d.spec.title.is_empty() {
            out.push(rectangle(
                owner,
                pid,
                212,
                l.title,
                text(
                    &d.spec.title,
                    s,
                    [200, 210, 230, 255],
                    UNWRAPPED_TEXT_LAYOUT,
                ),
                Some(l.title),
                order(2),
            ));
        }
        out.push(rectangle(
            owner,
            pid,
            213,
            l.message,
            text(&d.spec.message, s, [245, 245, 245, 255], SIMPLE_TEXT_LAYOUT),
            Some(l.message),
            order(3),
        ));
        for (i, (b, r)) in d.spec.buttons.iter().zip(&l.buttons).enumerate() {
            let focused = i == d.focus;
            let sub = 214 + i as u8 * 2;
            out.push(rectangle(
                owner,
                pid,
                sub,
                *r,
                Paint::Solid {
                    color: if focused {
                        [65, 85, 120, 255]
                    } else {
                        [60, 64, 72, 255]
                    },
                },
                clip,
                order(4),
            ));
            let pad = f32::from(s) * 0.5;
            let label = [
                r[0] + pad,
                r[1] + (r[3] - f32::from(s) * 1.2) / 2.,
                (r[2] - 2. * pad).max(1.),
                f32::from(s) * 1.3,
            ];
            out.push(rectangle(
                owner,
                pid,
                sub + 1,
                label,
                text(&b.label, s, [245, 245, 245, 255], UNWRAPPED_TEXT_LAYOUT),
                Some(*r),
                order(5),
            ));
        }
        Ok(out)
    }
}
