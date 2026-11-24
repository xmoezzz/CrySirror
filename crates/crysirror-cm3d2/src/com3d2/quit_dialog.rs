// SPDX-License-Identifier: MPL-2.0
use super::*;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Choice {
    Ok,
    Cancel,
}

impl Choice {
    fn from_path(path: &str) -> Option<Self> {
        match path.rsplit('/').next()? {
            "Ok" => Some(Self::Ok),
            "Cancel" => Some(Self::Cancel),
            _ => None,
        }
    }
}

pub(super) struct QuitDialog {
    pub first: usize,
    opened: u64,
    closing: Option<u64>,
    hover: Option<Choice>,
    pressed: Option<Choice>,
}

impl QuitDialog {
    pub fn show(widgets: &mut Vec<Widget>, mut dialog: Vec<Widget>, now: u64, message: &str) -> Result<Self> {
        for choice in [Choice::Ok, Choice::Cancel] {
            ensure!(
                dialog
                    .iter()
                    .any(|w| Choice::from_path(&w.path) == Some(choice) && w.collider.is_some()),
                "SystemDialog button collider"
            );
        }
        let label = dialog
            .iter_mut()
            .find(|w| w.path.ends_with("/Message"))
            .and_then(|w| w.label.as_mut())
            .context("SystemDialog message label")?;
        label.set_text(message);
        let top = widgets.iter().map(|w| w.depth).max().unwrap_or(0) + 1;
        let bottom = dialog.iter().map(|w| w.depth).min().unwrap_or(0);
        for w in &mut dialog {
            w.depth = top + (w.depth - bottom);
        }
        let first = widgets.len();
        widgets.extend(dialog);
        Ok(Self {
            first,
            opened: now,
            closing: None,
            hover: None,
            pressed: None,
        })
    }

    pub fn update(
        &mut self,
        widgets: &[Widget],
        input: InputFrame,
        released: u64,
        now: u64,
    ) -> Option<Choice> {
        if let Some(since) = self.closing {
            return (now.saturating_sub(since) >= 300).then_some(Choice::Cancel);
        }
        self.hover = widgets[self.first..].iter().find_map(|w| {
            let choice = Choice::from_path(&w.path)?;
            let [x, y, width, height] = w.collider?;
            let (px, py) = (input.pointer_x as f32, input.pointer_y as f32);
            (px >= x && px < x + width && py >= y && py < y + height).then_some(choice)
        });
        if input.buttons_pressed & buttons::MOUSE_LEFT != 0 {
            self.pressed = self.hover;
        }
        if released & buttons::MOUSE_LEFT != 0 && self.pressed.take() == self.hover {
            match self.hover {
                Some(Choice::Ok) => return Some(Choice::Ok),
                Some(Choice::Cancel) => self.closing = Some(now),
                None => {}
            }
        }
        None
    }

    pub fn opacity(&self, now: u64) -> u8 {
        let scale = |elapsed: u64| (elapsed as f32 / 300.).min(1.);
        let alpha = match self.closing {
            Some(since) => {
                scale(since.saturating_sub(self.opened)) * (1. - scale(now.saturating_sub(since)))
            }
            None => scale(now.saturating_sub(self.opened)),
        };
        (255. * alpha).round() as u8
    }

    pub fn cursor(&self, path: &str) -> bool {
        path.strip_suffix("/SelectCursor")
            .and_then(Choice::from_path)
            .is_some_and(|c| Some(c) == self.hover)
    }
}
