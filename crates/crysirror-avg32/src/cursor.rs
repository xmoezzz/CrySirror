// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Native selection over shared UI cursor definitions, never a composed screen.
use super::*;
const CURSOR: u32 = 0x4156_4352;
impl Avg32Vm {
    pub(super) fn publish_cursor(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let engine = self.engine.as_ref().expect("booted");
        let visible = engine.cursor_visible();
        let image = engine
            .system()
            .cursor
            .as_ref()
            .map(|cursor| cursor.image.clone());
        let mut ops = Vec::new();
        if let Some(image) = &image {
            if self
                .cursor_image
                .as_ref()
                .is_none_or(|old| !Arc::ptr_eq(old, image))
            {
                let frames: Vec<_> = image
                    .frames
                    .iter()
                    .map(|frame| {
                        json!({"width":frame.image.width,
                    "height":frame.image.height,"hotspot":frame.hotspot,"rgba":frame.image.rgba})
                    })
                    .collect();
                let steps: Vec<_> = image.steps.iter().map(|&(i, ms)| [i as u32, ms]).collect();
                ops.push(json!({"op":"define_cursor","id":CURSOR,"cursor":{"frames":frames,"steps":steps}}));
            }
        }
        if self.cursor_selected != Some(image.is_some()) {
            let select = if image.is_some() {
                json!({"kind":"custom","id":CURSOR})
            } else {
                json!({"kind":"standard","shape":"default"})
            };
            ops.push(json!({"op":"set_cursor","cursor":select}));
        }
        if self.cursor_visible != Some(visible) {
            ops.push(json!({"op":"set_cursor_visible","visible":visible}));
        }
        if !ops.is_empty() {
            self.submit(host, json!({"ui":ops}))?;
        }
        self.cursor_image = image;
        self.cursor_selected = Some(self.cursor_image.is_some());
        self.cursor_visible = Some(visible);
        Ok(())
    }
}
