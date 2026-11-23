// SPDX-License-Identifier: MPL-2.0
//! Caret/selection state for editable retained fields. Internally offsets are
//! UTF-8 byte boundaries between grapheme clusters. Pointer placement and
//! selection painting use the same measured visual lines.
use super::*;
use crysirror_core::service::{TextMeasurement, UNWRAPPED_TEXT_LAYOUT, buttons};
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, Default)]
pub(super) struct Editor {
    anchor: usize,
    head: usize,
    first_row: usize,
    scroll_x: f32,
    preferred_x: Option<f32>,
    layout: Option<Arc<Lines>>,
}

#[derive(Debug)]
struct Lines {
    rows: Vec<Vec<(usize, f32)>>,
    height: f32,
}

fn utf16_offset(text: &str, offset: i64) -> usize {
    let mut units = 0;
    for (at, g) in text.grapheme_indices(true) {
        units += g.encode_utf16().count() as i64;
        if units > offset {
            return at;
        }
    }
    text.len()
}

impl Editor {
    pub(super) fn select(&mut self, text: &str, start: i64, end: i64) {
        if start < 0 {
            self.anchor = self.head;
        } else {
            self.anchor = utf16_offset(text, start);
            self.head = if end < 0 {
                text.len()
            } else {
                utf16_offset(text, end)
            };
        }
        self.preferred_x = None;
    }

    pub(super) fn reset(&mut self, text: &str) {
        self.anchor = text.len();
        self.head = text.len();
        self.layout = None;
        self.preferred_x = None;
    }

    pub(super) fn invalidate_layout(&mut self) {
        self.layout = None;
    }

    fn range(&self) -> std::ops::Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub(super) fn replace(&mut self, s: &mut WidgetSpec, text: &str) {
        let r = self.range();
        let retained = s.text[..r.start].chars().count() + s.text[r.end..].chars().count();
        let mut available = (s.max_length as usize).saturating_sub(retained);
        let mut bytes = 0;
        for g in text.graphemes(true) {
            let count = g.chars().count();
            if count > available {
                break;
            }
            available -= count;
            bytes += g.len();
        }
        if bytes == 0 && !text.is_empty() {
            return;
        }
        let end = r.start + bytes;
        s.text.replace_range(r, &text[..bytes]);
        // Insertion/deletion may join clusters across the edit boundary.
        self.head = s
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|&i| i >= end)
            .unwrap_or(s.text.len());
        self.anchor = self.head;
        self.preferred_x = None;
        self.layout = None;
    }

    fn adjacent(&self, text: &str, left: bool) -> usize {
        if left {
            text[..self.head]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(i, _)| i)
        } else {
            self.head + text[self.head..].graphemes(true).next().map_or(0, str::len)
        }
    }

    pub(super) fn erase(&mut self, s: &mut WidgetSpec, left: bool) {
        if self.head == self.anchor {
            self.anchor = self.adjacent(&s.text, left);
        }
        if self.head != self.anchor {
            self.replace(s, "");
        }
    }

    fn place(&mut self, at: usize, extend: bool) {
        self.head = at;
        if !extend {
            self.anchor = at;
        }
    }

    pub(super) fn key(&mut self, s: &mut WidgetSpec, frame: InputFrame, edges: u64) {
        let shift = frame.buttons_down & buttons::SHIFT != 0;
        let ctrl = frame.buttons_down & buttons::CTRL != 0;
        if ctrl && edges & buttons::KEY_A != 0 {
            self.select(&s.text, 0, -1);
        } else if edges & buttons::DELETE != 0 {
            self.erase(s, false);
        } else if edges & (buttons::LEFT | buttons::RIGHT) != 0 {
            let left = edges & buttons::LEFT != 0;
            let r = self.range();
            let at = if !shift && !r.is_empty() {
                if left { r.start } else { r.end }
            } else {
                self.adjacent(&s.text, left)
            };
            self.place(at, shift);
            self.preferred_x = None;
        } else if let Some(layout) = &self.layout {
            let row = layout.row_of(self.head);
            let line = &layout.rows[row];
            if edges & (buttons::HOME | buttons::END) != 0 {
                let at = match (edges & buttons::HOME != 0, ctrl) {
                    (true, true) => 0,
                    (false, true) => s.text.len(),
                    (true, false) => line[0].0,
                    (false, false) => line.last().unwrap().0,
                };
                self.place(at, shift);
                self.preferred_x = None;
            } else if edges & (buttons::UP | buttons::DOWN) != 0 {
                let x = *self
                    .preferred_x
                    .get_or_insert_with(|| x_at(line, self.head));
                let next = if edges & buttons::UP != 0 {
                    row.saturating_sub(1)
                } else {
                    (row + 1).min(layout.rows.len() - 1)
                };
                self.place(nearest(&layout.rows[next], x), shift);
            }
        }
    }

    pub(super) fn point(&mut self, x: f32, y: f32, extend: bool) {
        if let Some(layout) = &self.layout {
            let row = (self.first_row as isize + (y / layout.height).floor() as isize)
                .clamp(0, layout.rows.len() as isize - 1) as usize;
            self.place(nearest(&layout.rows[row], x + self.scroll_x), extend);
            self.preferred_x = None;
        }
    }

    fn reveal(&mut self, spec: &WidgetSpec) {
        let Some(l) = &self.layout else { return };
        let row = l.row_of(self.head);
        let height = (spec.rect[3] - 2. * f32::from(spec.text_inset[1])).max(1.);
        let visible = ((height / l.height).floor() as usize).max(1);
        self.first_row = self
            .first_row
            .min(row)
            .max((row + 1).saturating_sub(visible));
        let x = x_at(&l.rows[row], self.head);
        let width = (spec.rect[2] - 2. * f32::from(spec.text_inset[0]) - 2.).max(1.);
        self.scroll_x = self.scroll_x.min(x).max(x - width).max(0.);
    }

    pub(super) fn paint(
        &self,
        layout: &Layout,
        s: &WidgetSpec,
        focused: bool,
        out: &mut Vec<Primitive>,
        order: [i32; 2],
    ) -> bool {
        let Some(lines) = &self.layout else {
            return false;
        };
        let [pad_x, pad_y] = s.text_inset.map(f32::from);
        let inner = [
            layout.rect[0] + pad_x,
            layout.rect[1] + pad_y,
            (layout.rect[2] - 2. * pad_x).max(1.),
            (layout.rect[3] - 2. * pad_y).max(1.),
        ];
        let Some(clip) = intersect(layout.clip, inner) else {
            return true;
        };
        let x = layout.rect[0] + f32::from(s.text_inset[0]) - self.scroll_x;
        let top = layout.rect[1] + f32::from(s.text_inset[1]);
        let range = self.range();
        for (i, line) in lines.rows.iter().enumerate().skip(self.first_row) {
            let y = top + (i - self.first_row) as f32 * lines.height;
            if y >= clip[1] + clip[3] {
                break;
            }
            let left = line[0].0;
            let right = line.last().unwrap().0;
            let mut draw = |sub, rect, paint| {
                out.push(rectangle(
                    layout.id.owner,
                    layout.id.id,
                    sub,
                    rect,
                    paint,
                    Some(clip),
                    order,
                ))
            };
            if focused && range.start <= right && range.end > left && !range.is_empty() {
                let a = x_at(line, range.start.max(left));
                let b = x_at(line, range.end.min(right));
                draw(
                    1,
                    [x + a, y, (b - a).max(1.), lines.height],
                    Paint::Solid {
                        color: [118, 183, 250, 255],
                    },
                );
            }
            if left < right {
                // A long unwrapped field still rasterizes only the visible
                // span. Include adjacent clusters for glyph ink overhang.
                let first = line
                    .partition_point(|(_, at)| x + at < clip[0])
                    .saturating_sub(2);
                let last = (line.partition_point(|(_, at)| x + at <= clip[0] + clip[2]) + 1)
                    .min(line.len() - 1);
                let (from, pen) = line[first];
                let (to, end) = line[last];
                draw(
                    2,
                    [
                        x + pen,
                        y,
                        (end - pen).ceil().max(1.) + 2.,
                        lines.height.ceil() + 2.,
                    ],
                    Paint::Text {
                        text: s.text[from..to].into(),
                        font: s.font.clone().unwrap_or_default(),
                        size: s.text_size,
                        color: s.text_color,
                        layout: UNWRAPPED_TEXT_LAYOUT.into(),
                    },
                );
            }
            if focused && range.is_empty() && i == lines.row_of(self.head) {
                draw(
                    3,
                    [x + x_at(line, self.head), y, 1.5, lines.height],
                    Paint::Solid {
                        color: s.text_color,
                    },
                );
            }
        }
        true
    }
}

fn x_at(line: &[(usize, f32)], at: usize) -> f32 {
    line[line.partition_point(|(i, _)| *i <= at).saturating_sub(1)].1
}
fn nearest(line: &[(usize, f32)], x: f32) -> usize {
    let right = line.partition_point(|(_, pos)| *pos < x);
    if right == 0 {
        return line[0].0;
    }
    let left = line[right - 1];
    line.get(right)
        .filter(|(_, p)| *p - x < x - left.1)
        .map_or(left.0, |p| p.0)
}

impl Lines {
    fn row_of(&self, index: usize) -> usize {
        self.rows
            .partition_point(|row| row[0].0 <= index)
            .saturating_sub(1)
    }

    fn measure(s: &WidgetSpec, api: &impl TextMeasurement) -> Result<Self> {
        let font = s.font.as_deref().context("missing editor font")?;
        let size = f32::from(s.text_size);
        let height = api.measure_simple(font, size, "")?.line_height.max(1.);
        let width = (s.rect[2] - 2. * f32::from(s.text_inset[0])).max(1.);
        let mut rows = Vec::new();
        let mut line = vec![(0, 0.)];
        let mut previous: Option<(usize, f32)> = None;
        for (i, g) in s.text.grapheme_indices(true) {
            let end = i + g.len();
            if matches!(g, "\n" | "\r" | "\r\n") {
                rows.push(line);
                line = vec![(end, 0.)];
                previous = None;
                continue;
            }
            let advance = api.measure_simple(font, size, g)?.advance;
            let mut x = line.last().unwrap().1;
            if let Some((start, prev_width)) = previous {
                let pair = api.measure_simple(font, size, &s.text[start..end])?.advance;
                x += pair - prev_width - advance;
            }
            if s.multiline && !s.no_wrap && line.len() > 1 && x + advance > width {
                rows.push(line);
                line = vec![(i, 0.)];
                x = 0.;
            }
            line.push((end, (x + advance).max(line.last().unwrap().1)));
            previous = Some((i, advance));
        }
        rows.push(line);
        Ok(Self { rows, height })
    }
}

impl UiSystem {
    /// Handle desktop editing keys in event order with IME/text commits.
    /// Frame-driven hosts dispatch the same operations from tick().
    pub fn focused_editor_key(&mut self, key: u64, modifiers: u64) -> bool {
        if !self.focused_text_field() || !self.dialogs.is_empty() {
            return false;
        }
        let Some(id) = self.focus else { return false };
        let Some(w) = self
            .clients
            .get_mut(&id.owner)
            .and_then(|c| c.get_mut(&id.id))
        else {
            return false;
        };
        let Some(editor) = &mut w.editor else {
            return false;
        };
        if key == buttons::BACKSPACE {
            editor.erase(&mut w.spec, true);
        } else if key
            & (buttons::LEFT
                | buttons::RIGHT
                | buttons::UP
                | buttons::DOWN
                | buttons::DELETE
                | buttons::HOME
                | buttons::END)
            != 0
            || key == buttons::KEY_A && modifiers & buttons::CTRL != 0
        {
            editor.key(
                &mut w.spec,
                InputFrame {
                    buttons_down: key | modifiers,
                    ..Default::default()
                },
                key,
            );
        } else {
            return false;
        }
        true
    }
    /// Supply client-scoped font metrics for the focused range editor.
    /// Font-free simulations still support keyboard selection and insertion.
    pub fn prepare_text_editor<T: TextMeasurement>(
        &mut self,
        lookup: impl FnOnce(u32, &str) -> Option<T>,
    ) -> Result<()> {
        let Some(id) = self.focus else { return Ok(()) };
        let Some(w) = self
            .clients
            .get_mut(&id.owner)
            .and_then(|m| m.get_mut(&id.id))
        else {
            return Ok(());
        };
        let Some(editor) = &mut w.editor else {
            return Ok(());
        };
        if editor.layout.is_none() {
            let Some(api) = lookup(id.owner, w.spec.font.as_deref().unwrap_or("")) else {
                return Ok(());
            };
            editor.layout = Some(Arc::new(Lines::measure(&w.spec, &api)?));
        }
        editor.reveal(&w.spec);
        Ok(())
    }
}
