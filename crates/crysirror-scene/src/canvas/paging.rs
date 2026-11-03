//! Logical Canvas pages independent of a GPU texture dimension.
use anyhow::{Result, ensure};
/// Coordinates remain small enough for exact integer positions in f32.
pub const MAX_LOGICAL_EDGE: u32 = 32768;
/// Small immutable captures keep visible-page working sets bounded.
pub const PAGE_EDGE: u32 = 1024;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasPage {
    pub region: [u32; 4],
    /// One-pixel source gutter prevents seams under linear sampling.
    pub capture: [u32; 4],
    pub uv: [f32; 4],
}
pub fn pages(region: [u32; 4], canvas: [u32; 2]) -> Result<Vec<CanvasPage>> {
    let [x, y, w, h] = region;
    ensure!(
        canvas.iter().all(|v| *v > 0 && *v <= MAX_LOGICAL_EDGE)
            && w > 0
            && h > 0
            && x.checked_add(w).is_some_and(|r| r <= canvas[0])
            && y.checked_add(h).is_some_and(|b| b <= canvas[1]),
        "Canvas page bounds"
    );
    let mut out = Vec::new();
    for top in (y..y + h).step_by(PAGE_EDGE as usize) {
        for left in (x..x + w).step_by(PAGE_EDGE as usize) {
            let right = left.saturating_add(PAGE_EDGE).min(x + w);
            let bottom = top.saturating_add(PAGE_EDGE).min(y + h);
            let cx = left.saturating_sub(1);
            let cy = top.saturating_sub(1);
            let cr = right.saturating_add(1).min(canvas[0]);
            let cb = bottom.saturating_add(1).min(canvas[1]);
            let capture = [cx, cy, cr - cx, cb - cy];
            out.push(CanvasPage {
                region: [left, top, right - left, bottom - top],
                capture,
                uv: [
                    (left - cx) as f32 / capture[2] as f32,
                    (top - cy) as f32 / capture[3] as f32,
                    (right - cx) as f32 / capture[2] as f32,
                    (bottom - cy) as f32 / capture[3] as f32,
                ],
            });
        }
    }
    Ok(out)
}
