// SPDX-License-Identifier: MPL-2.0
//! Presents the engine UI cursor selection (`Ui.setCursor`) on the winit
//! window: standard shapes map to `CursorIcon`; client cursor images become
//! OS custom cursors, cached per definition revision and stepped through
//! their animation schedule.
use crysirror_scene::ui::{
    UiSystem,
    cursor::{CursorSelect, StandardCursor},
};
use std::{collections::HashMap, time::Instant};
use winit::{
    event_loop::ActiveEventLoop,
    window::{CursorIcon, CustomCursor, Window},
};

#[derive(Default)]
pub struct CursorPresenter {
    revision: u64,
    cache: HashMap<(u32, u32), Vec<CustomCursor>>,
    applied: Option<(u32, CursorSelect)>,
    /// Animation: current step index and when it started.
    step: usize,
    step_started: Option<Instant>,
    shown_frame: Option<usize>,
}

fn icon(shape: StandardCursor) -> CursorIcon {
    match shape {
        StandardCursor::Default => CursorIcon::Default,
        StandardCursor::Crosshair => CursorIcon::Crosshair,
        StandardCursor::Text => CursorIcon::Text,
        StandardCursor::VerticalText => CursorIcon::VerticalText,
        StandardCursor::Move => CursorIcon::Move,
        StandardCursor::NeswResize => CursorIcon::NeswResize,
        StandardCursor::NsResize => CursorIcon::NsResize,
        StandardCursor::NwseResize => CursorIcon::NwseResize,
        StandardCursor::EwResize => CursorIcon::EwResize,
        StandardCursor::Wait => CursorIcon::Wait,
        StandardCursor::Progress => CursorIcon::Progress,
        StandardCursor::Grabbing => CursorIcon::Grabbing,
        StandardCursor::NoDrop => CursorIcon::NoDrop,
        StandardCursor::ColResize => CursorIcon::ColResize,
        StandardCursor::RowResize => CursorIcon::RowResize,
        StandardCursor::Copy => CursorIcon::Copy,
        StandardCursor::NotAllowed => CursorIcon::NotAllowed,
        StandardCursor::Help => CursorIcon::Help,
        StandardCursor::Pointer => CursorIcon::Pointer,
    }
}

impl CursorPresenter {
    /// Apply the current selection; call once per presented frame.
    pub fn sync(&mut self, el: &ActiveEventLoop, window: &Window, ui: &UiSystem) {
        let cursors = &ui.cursors;
        if cursors.revision != self.revision {
            self.revision = cursors.revision;
            self.cache.clear();
            self.applied = None;
        }
        let selected = cursors.selected;
        let changed = selected != self.applied;
        if changed {
            self.applied = selected;
            self.step = 0;
            self.step_started = Some(Instant::now());
            self.shown_frame = None;
        }
        match selected {
            None => {
                if changed {
                    window.set_cursor(CursorIcon::Default);
                }
            }
            Some((_, CursorSelect::Standard { shape })) => {
                if changed {
                    window.set_cursor(icon(shape));
                }
            }
            Some((client, CursorSelect::Custom { id })) => {
                let Some(spec) = cursors.get(client, id) else {
                    return;
                };
                if !self.cache.contains_key(&(client, id)) {
                    let mut built = Vec::with_capacity(spec.frames.len());
                    for f in &spec.frames {
                        match CustomCursor::from_rgba(
                            f.rgba.clone(),
                            f.width as u16,
                            f.height as u16,
                            f.hotspot[0] as u16,
                            f.hotspot[1] as u16,
                        ) {
                            Ok(source) => built.push(el.create_custom_cursor(source)),
                            Err(e) => {
                                log::warn!("custom cursor {id}: {e}");
                                return;
                            }
                        }
                    }
                    self.cache.insert((client, id), built);
                }
                // Advance the animation by wall time.
                if spec.steps.len() > 1 {
                    let now = Instant::now();
                    let mut started = self.step_started.unwrap_or(now);
                    loop {
                        let ms = u64::from(spec.steps[self.step][1].max(1));
                        let due = started + std::time::Duration::from_millis(ms);
                        if now < due {
                            break;
                        }
                        started = due;
                        self.step = (self.step + 1) % spec.steps.len();
                    }
                    self.step_started = Some(started);
                }
                let frame = spec.steps[self.step.min(spec.steps.len() - 1)][0] as usize;
                if self.shown_frame != Some(frame) {
                    if let Some(cursor) = self.cache[&(client, id)].get(frame) {
                        window.set_cursor(cursor.clone());
                        self.shown_frame = Some(frame);
                    }
                }
            }
        }
    }
}
