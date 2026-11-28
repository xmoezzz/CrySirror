// SPDX-License-Identifier: MPL-2.0
//! RLD v1 enterClip on the active CG surface, using TmAsyncMagnify's integer edges.
use anyhow::{Context, Result, ensure};
use crysirror_core::{service::HostServices, state::ClientId};
use serde_json::{Value, json};

pub(super) struct Clip {
    from: [i32; 4],
    to: [i32; 4],
    shown: [i32; 4],
    source: [u32; 2],
    asset: String,
    started: Option<u64>,
    duration: u32,
    mode: i32,
}

fn edges([x, y, w, h]: [i32; 4], [sw, sh]: [u32; 2]) -> Result<[i32; 4]> {
    ensure!(
        x >= 0
            && y >= 0
            && w > 0
            && h > 0
            && i64::from(x) + i64::from(w) <= i64::from(sw)
            && i64::from(y) + i64::from(h) <= i64::from(sh)
            && sw <= i32::MAX as u32
            && sh <= i32::MAX as u32,
        "unsupported ExHibit clip outside source"
    );
    Ok([x, y, x + w, y + h])
}

// The v1 opening mode-9 record selects the oversized two-axis source path
// in UxAdvSystem::draw (0x37780), not a scaled nominal UV space. Its x86
// full-destination helper (0x8cab0 -> 0x8cb70) intersects source and destination
// independently; equal intersected sizes reduce to a direct copy.
pub(super) fn is_opening_draw(
    abi: u32,
    script: &str,
    pc: usize,
    id: i32,
    duration: u64,
    crop: [i32; 4],
    retain_clip: bool,
) -> bool {
    abi == 1
        && script.eq_ignore_ascii_case("F00A1101_2")
        && pc == 0x130
        && id == 1239
        && duration == 500
        && crop == [0, 0, 1600, 1200]
        && !retain_clip
}

pub(super) fn draw_uv(
    crop: [i32; 4],
    source: [u32; 2],
    canvas: [u32; 2],
    opening: bool,
) -> Result<[f32; 4]> {
    let [x, y, w, h] = crop;
    ensure!(
        x >= 0 && y >= 0 && w > 0 && h > 0,
        "invalid ExHibit CG crop"
    );
    let [x, y, w, h] = crop.map(|n| n as u32);
    let [sw, sh] = source;
    let mut right = x.checked_add(w).context("CG crop width overflow")?;
    let mut bottom = y.checked_add(h).context("CG crop height overflow")?;
    if opening {
        ensure!(
            crop == [0, 0, 1600, 1200] && source == [800, 600] && source == canvas,
            "unsupported ExHibit opening CG source {source:?} / canvas {canvas:?}"
        );
        right = right.min(sw);
        bottom = bottom.min(sh);
    }
    ensure!(
        right <= sw && bottom <= sh && [right - x, bottom - y] == canvas,
        "unsupported ExHibit CG crop {crop:?} on {sw}x{sh}"
    );
    Ok([
        x as f32 / sw as f32,
        y as f32 / sh as f32,
        right as f32 / sw as f32,
        bottom as f32 / sh as f32,
    ])
}

pub(super) fn draw_commands(
    paint: Value,
    [width, height]: [u32; 2],
    uv: Option<[f32; 4]>,
    duration: u64,
) -> Value {
    let mut layers = Vec::new();
    if duration != 0 {
        layers.push(json!({
            "op": "transition",
            "transition": {
                "effect": {"kind": "crossfade"},
                "duration_ms": duration,
                "easing": {"progress": {"shape": {"kind": "steps", "segments": 16}}},
            }
        }));
    }
    let mut layer = json!({
        "id": 1,
        "order": [0, 0],
        "rect": [0, 0, width, height],
        "paint": paint,
    });
    if let Some(uv) = uv {
        layer["uv"] = json!(uv);
    }
    layers.push(json!({"op": "put", "layer": layer}));
    json!({"layers": layers})
}

pub(super) fn solid_paint(id: i32) -> Value {
    // draw converts the signed CG word to COLORREF, then 0x8b1f0
    // writes its three channels to an opaque 24-bit surface.
    let [b, g, r, _] = id.to_le_bytes();
    json!({"kind": "solid", "color": [r, g, b, 255]})
}

pub(super) fn offset(delta: i32, elapsed: u32, duration: u32, mode: i32) -> i32 {
    if elapsed >= duration {
        return delta;
    }
    if elapsed == 0 {
        return 0;
    }
    let n = i64::from(delta).abs();
    let step = match mode {
        // FCAsyncEffect::calcInt squares the two unsigned clock words before widening.
        1 => {
            n * i64::from(elapsed.wrapping_mul(elapsed))
                / i64::from(duration.wrapping_mul(duration))
        }
        2 => ((elapsed as f64 * n as f64 * n as f64 / duration as f64).sqrt()) as i64,
        3 => (n as f64 * (elapsed as f64).ln() / (duration as f64).ln()) as i64,
        _ => n * i64::from(elapsed) / i64::from(duration),
    };
    delta.signum() * step.clamp(0, n) as i32
}

impl Clip {
    pub fn new(
        asset: String,
        source: [u32; 2],
        from: [i32; 4],
        to: [i32; 4],
        duration: u32,
        mode: i32,
    ) -> Result<Self> {
        ensure!(
            mode != 1 || duration == 0 || duration.wrapping_mul(duration) != 0,
            "invalid ExHibit quadratic clip duration"
        );
        let from = edges(from, source)?;
        let to = edges(to, source)?;
        Ok(Self {
            from,
            to,
            shown: from,
            asset,
            source,
            started: None,
            duration,
            mode,
        })
    }
    fn sample(&self, elapsed: u32) -> [i32; 4] {
        std::array::from_fn(|i| {
            self.from[i] + offset(self.to[i] - self.from[i], elapsed, self.duration, self.mode)
        })
    }
    pub fn crop(&self) -> [i32; 4] {
        let [l, t, r, b] = self.shown;
        [l, t, r - l, b - t]
    }
    pub fn present(
        &mut self,
        h: &mut dyn HostServices,
        client: ClientId,
        canvas: [u32; 2],
    ) -> Result<bool> {
        let now = h.now_ms();
        let elapsed = now
            .saturating_sub(*self.started.get_or_insert(now))
            .min(u64::from(self.duration)) as u32;
        let next = self.sample(elapsed);
        if next != self.shown {
            let [sw, sh] = self.source;
            let uv = std::array::from_fn::<_, 4, _>(|i| next[i] as f32 / [sw, sh][i % 2] as f32);
            let [w, height] = canvas;
            let payload = json!({"layers":[{"op":"put","layer":{"id":1,"order":[0,0],"rect":[0,0,w,height],"uv":uv,"paint":{"kind":"image","asset":self.asset}}}]});
            h.submit_engine(client, 1, &serde_json::to_vec(&payload)?)?;
            self.shown = next;
        }
        Ok(elapsed == self.duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opening_oversized_draw_intersects_to_full_source_not_scaled_uv() -> Result<()> {
        let authored = [0, 0, 1600, 1200];
        let screen = [800, 600];
        assert!(is_opening_draw(
            1,
            "f00a1101_2",
            0x130,
            1239,
            500,
            authored,
            false
        ));
        assert_eq!(draw_uv(authored, screen, screen, true)?, [0., 0., 1., 1.]);
        assert!(draw_uv(authored, screen, screen, false).is_err());
        assert!(draw_uv(authored, [1600, 1200], screen, true).is_err());
        assert!(draw_uv([1, 0, 1600, 1200], screen, screen, true).is_err());
        assert!(draw_uv(authored, screen, [640, 480], true).is_err());
        assert!(!is_opening_draw(
            3,
            "F00A1101_2",
            0x130,
            1239,
            500,
            authored,
            false
        ));
        assert!(!is_opening_draw(
            1,
            "F00A1101_2",
            0x134,
            1239,
            500,
            authored,
            false
        ));
        assert!(!is_opening_draw(
            1,
            "F00A1101_2",
            0x130,
            1239,
            500,
            authored,
            true
        ));
        assert!(!is_opening_draw(
            1, "other", 0x130, 1239, 500, authored, false
        ));
        assert_eq!(
            draw_uv([10, 30, 800, 600], [1000, 750], screen, false)?,
            [0.01, 0.04, 0.81, 0.84]
        );
        assert!(draw_uv([0, 0, 799, 600], screen, screen, false).is_err());
        Ok(())
    }

    #[test]
    fn opening_draw_replaces_only_the_background_on_the_shared_compositor() -> Result<()> {
        let asset = "res/g/ev/12/1239.gyu";
        let canvas = [800, 600];
        let uv = draw_uv([0, 0, 1600, 1200], canvas, canvas, true)?;
        let command = draw_commands(json!({"kind":"image","asset":asset}), canvas, Some(uv), 500);
        let ops = command["layers"].as_array().context("draw operations")?;
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0]["transition"]["duration_ms"], 500);
        assert_eq!(
            ops[0]["transition"]["easing"]["progress"]["shape"]["segments"],
            16
        );
        assert_eq!(ops[1]["layer"]["id"], 1);
        assert_eq!(ops[1]["layer"]["rect"], json!([0, 0, 800, 600]));
        assert_eq!(ops[1]["layer"]["uv"], json!([0., 0., 1., 1.]));
        assert_eq!(ops[1]["layer"]["paint"]["asset"], asset);
        Ok(())
    }

    #[test]
    fn native_title_pan_samples_integer_source_edges() -> Result<()> {
        let clip = Clip::new(
            "cg".into(),
            [800, 1100],
            [0, 500, 800, 600],
            [0, 0, 800, 600],
            2500,
            1,
        )?;
        assert_eq!(clip.sample(0), [0, 500, 800, 1100]);
        assert_eq!(clip.sample(1250), [0, 375, 800, 975]);
        assert_eq!(clip.sample(2499), [0, 1, 800, 601]);
        assert_eq!(clip.sample(2500), [0, 0, 800, 600]);
        assert_eq!(offset(300, 25, 100, 2), 150);
        assert_eq!(offset(-300, 10, 100, 3), -150);
        Ok(())
    }
}
