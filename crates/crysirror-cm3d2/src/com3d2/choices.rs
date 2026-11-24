// SPDX-License-Identifier: MPL-2.0
use super::*;
use crysirror_unity::reader::Reader;

#[derive(Default)]
pub(super) struct Layout {
    step: f32,
    x: f32,
    clip: [f32; 4],
    pivot: i32,
}

impl Layout {
    pub fn read(&mut self, class: &str, body: &[u8], pos: [f32; 3]) -> Result<()> {
        let mut r = Reader::new(body);
        match class {
            "UIGrid" => {
                r.seek(32)?;
                ensure!(r.i32()? == 1, "unsupported choice grid arrangement");
                r.seek(52)?;
                self.step = r.f32()?;
                self.x = 960. + pos[0];
            }
            "UIPanel" => {
                r.seek(172)?;
                let [x, y, w, h] = r.f32s::<4>()?;
                r.seek(204)?;
                let [dx, dy] = r.f32s::<2>()?;
                self.clip = [
                    960. + pos[0] + x + dx - w / 2.,
                    540. - pos[1] - y - dy - h / 2.,
                    w,
                    h,
                ];
            }
            "UIScrollView" => {
                r.seek(100)?;
                self.pivot = r.i32()?;
            }
            _ => {}
        }
        Ok(())
    }
}
pub(super) struct Choices {
    pub first: usize,
    stride: usize,
    options: Vec<(String, String, bool)>,
    hits: Vec<[f32; 4]>,
    clip: [f32; 4],
    focus: Option<usize>,
    pressed: Option<usize>,
    pointer: Option<[i32; 2]>,
    selected: Option<(usize, u64)>,
    step: f32,
    scroll: f32,
    max_scroll: f32,
}
fn inside(r: [f32; 4], p: [i32; 2]) -> bool {
    let [x, y, w, h] = r;
    p[0] as f32 >= x && (p[0] as f32) < x + w && p[1] as f32 >= y && (p[1] as f32) < y + h
}
impl Choices {
    pub fn show(
        widgets: &mut Vec<Widget>,
        template: Vec<Widget>,
        options: Vec<(String, String, bool)>,
        layout: &Layout,
    ) -> Result<Self> {
        ensure!(
            !options.is_empty() && layout.step > 0. && layout.clip[3] > 0.,
            "invalid choice grid"
        );
        let button = template
            .iter()
            .find_map(|w| w.collider)
            .context("choice button collider")?;
        let first = widgets.len();
        let stride = template.len();
        let top = template
            .iter()
            .map(|w| w.rect[1])
            .fold(f32::INFINITY, f32::min);
        let bottom = template
            .iter()
            .map(|w| w.rect[1] + w.rect[3])
            .fold(f32::NEG_INFINITY, f32::max);
        let total = bottom - top + layout.step * (options.len() - 1) as f32;
        let remaining = layout.clip[3] - total;
        let align = match layout.pivot {
            0..=2 => 0.,
            3..=5 => 0.5,
            6..=8 => 1.,
            p => bail!("unknown choice content pivot {p}"),
        };
        let start = layout.clip[1] + remaining.max(0.) * align - top;
        let mut hits = Vec::new();
        for (i, (text, _, _)) in options.iter().enumerate() {
            let dx = layout.x - 960.;
            let dy = start + i as f32 * layout.step;
            hits.push([button[0] + dx, button[1] + dy, button[2], button[3]]);
            for mut w in template.iter().cloned() {
                w.rect[0] += dx;
                w.rect[1] += dy;
                if w.path.ends_with("/Option")
                    && let Some(l) = &mut w.label
                {
                    l.set_text(text);
                }
                w.depth += 100;
                widgets.push(w);
            }
        }
        let focus = options.iter().position(|o| o.2);
        Ok(Self {
            first,
            stride,
            options,
            hits,
            clip: layout.clip,
            focus,
            pressed: None,
            pointer: None,
            selected: None,
            step: layout.step,
            scroll: 0.,
            max_scroll: (-remaining).max(0.),
        })
    }
    pub fn update(&mut self, input: InputFrame, released: u64, now: u64) -> Option<&str> {
        if let Some((i, start)) = self.selected {
            return (now.saturating_sub(start) >= 1000).then(|| self.options[i].1.as_str());
        }
        if input.buttons_pressed & buttons::WHEEL_UP != 0 {
            self.scroll = (self.scroll - self.step).max(0.);
        }
        if input.buttons_pressed & buttons::WHEEL_DOWN != 0 {
            self.scroll = (self.scroll + self.step).min(self.max_scroll);
        }
        let p = [input.pointer_x, input.pointer_y];
        let hit = self.hits.iter().enumerate().find_map(|(i, &mut_rect)| {
            let mut r = mut_rect;
            r[1] -= self.scroll;
            (inside(self.clip, p) && inside(r, p) && self.options[i].2).then_some(i)
        });
        if self.pointer != Some(p) {
            self.pointer = Some(p);
            if hit.is_some() {
                self.focus = hit;
            }
        }
        if let Some(i) = self.focus {
            let dir = if input.buttons_pressed & buttons::DOWN != 0 {
                1
            } else if input.buttons_pressed & buttons::UP != 0 {
                -1
            } else {
                0
            };
            if dir != 0 {
                let n = self.options.len() as isize;
                self.focus = (1..=n)
                    .map(|k| (i as isize + dir * k).rem_euclid(n) as usize)
                    .find(|&j| self.options[j].2);
                if let Some(j) = self.focus {
                    let [_, y, _, h] = self.hits[j];
                    self.scroll = self
                        .scroll
                        .max(y + h - self.clip[1] - self.clip[3])
                        .min(y - self.clip[1])
                        .clamp(0., self.max_scroll);
                }
            }
        }
        if input.buttons_pressed & buttons::MOUSE_LEFT != 0 {
            self.pressed = hit;
        }
        let confirm = if released & buttons::MOUSE_LEFT != 0 && self.pressed.take() == hit {
            hit
        } else if input.buttons_pressed & buttons::ENTER != 0 {
            self.focus
        } else {
            None
        };
        if let Some(i) = confirm {
            self.selected = Some((i, now));
        }
        None
    }
    pub fn draw(&self, index: usize, path: &str, now: u64) -> (f32, [f32; 4], bool, u8) {
        let row = (index - self.first) / self.stride;
        let visible = !path.ends_with("/SelectCursor") || self.focus == Some(row);
        let alpha = self.selected.map_or(255, |(selected, start)| {
            let ms = if selected == row { 1000. } else { 300. };
            (255. * (1. - (now.saturating_sub(start) as f32 / ms).min(1.))).round() as u8
        });
        (-self.scroll, self.clip, visible, alpha)
    }
    pub fn labels(&self) -> Vec<&str> {
        self.options.iter().map(|r| r.0.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu() -> Choices {
        Choices {
            first: 0,
            stride: 1,
            options: vec![
                ("A".into(), "*a".into(), true),
                ("B".into(), "*b".into(), false),
                ("C".into(), "*c".into(), true),
            ],
            hits: vec![
                [0., 0., 100., 40.],
                [0., 50., 100., 40.],
                [0., 100., 100., 40.],
            ],
            clip: [0., 0., 100., 100.],
            focus: Some(0),
            pressed: None,
            pointer: None,
            selected: None,
            step: 50.,
            scroll: 0.,
            max_scroll: 40.,
        }
    }

    fn input(y: i32, buttons: u64) -> InputFrame {
        InputFrame {
            pointer_x: 20,
            pointer_y: y,
            buttons_down: buttons,
            buttons_pressed: buttons,
        }
    }

    #[test]
    fn choice_click_requires_matching_enabled_row_and_waits_for_callback() {
        let mut choices = menu();
        let left = buttons::MOUSE_LEFT;
        choices.update(input(60, left), 0, 0);
        choices.update(input(60, 0), left, 16);
        choices.update(input(20, left), 0, 32);
        choices.update(input(130, 0), left, 48);
        assert!(choices.selected.is_none());
        choices.update(input(20, left), 0, 64);
        choices.update(input(20, 0), left, 80);
        assert_eq!(choices.draw(1, "Option", 380).3, 0);
        assert!(choices.draw(0, "Option", 380).3 > 0);
        assert!(choices.update(input(20, 0), 0, 1079).is_none());
        assert_eq!(choices.update(input(20, 0), 0, 1080), Some("*a"));
    }

    #[test]
    fn choice_keyboard_skips_disabled_and_scrolls_focused_row_into_view() {
        let mut choices = menu();
        choices.update(input(200, buttons::DOWN), 0, 0);
        assert_eq!(choices.focus, Some(2));
        assert_eq!(choices.draw(2, "Option", 0).0, -40.);
        choices.update(input(200, buttons::ENTER), 0, 16);
        assert_eq!(choices.update(input(200, 0), 0, 1016), Some("*c"));
    }
}
