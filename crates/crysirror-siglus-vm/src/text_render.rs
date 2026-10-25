// SPDX-License-Identifier: MPL-2.0
// Siglus's fixed-cell editbox metric, retained without fonts/devices.
pub fn editbox_cell_width_px(ch: char, font_px: i32) -> i32 {
    let full = font_px.max(1);
    if ch.is_ascii() || ('\u{ff61}'..='\u{ff9f}').contains(&ch) {
        ((full + 1) / 2).max(1)
    } else {
        full
    }
}
