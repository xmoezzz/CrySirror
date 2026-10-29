// SPDX-License-Identifier: MPL-2.0
//! RealLive `#SEL` reveal transitions as rule images. The reveal functions
//! are copied from siglus_rs `crates/reallive/src/effects.rs` (commit
//! 5f358ef1360b0439e92aed2b749e8f25d8721954, MPL-2.0): every pixel gets the
//! progress at which the new picture appears there. [`reveal_rule`] turns
//! that into a grayscale rule (bright = early) for the engine's mask
//! transition. Styles that move pictures (scrolls, slides, zooms) are not
//! reveals and return `None`.


#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Transition {
    pub time: i32,
    pub style: i32,
    pub direction: i32,
    pub interpolation: i32,
    pub xsize: i32,
    pub ysize: i32,
    pub a: i32,
    pub b: i32,
    pub opacity: i32,
    pub c: i32,
}

impl Transition {
    /// From the 16 values of `#SEL.nnn` or the long form of `grpOpen`:
    /// `x1 y1 x2 y2 dx dy time style direction interpolation xsize ysize a
    /// b opacity c`.
    pub fn from_sel(values: &[i32]) -> Self {
        let get = |i: usize| values.get(i).copied().unwrap_or(0);
        Self {
            time: get(6),
            style: get(7),
            direction: get(8),
            interpolation: get(9),
            xsize: get(10),
            ysize: get(11),
            a: get(12),
            b: get(13),
            opacity: if values.len() > 14 { get(14) } else { 255 },
            c: get(15),
        }
    }

    pub fn instant() -> Self {
        Self {
            opacity: 255,
            ..Self::default()
        }
    }
}

/// Deterministic per-cell noise in `[0, 1)`.
fn noise(seed: i64) -> f64 {
    let mut z = (seed as u64).wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

const BAYER4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

type Reveal = Box<dyn Fn(i32, i32) -> (f64, f64)>;

/// Linear sweep along an axis given a pixel position `p` in `0..len`.
fn sweep(p: f64, len: f64, soft: f64) -> (f64, f64) {
    let span = len + soft;
    ((p + soft / 2.0) / span, soft / span)
}

fn reveal_function(tr: &Transition, w: i32, h: i32, soft: f64) -> Option<Reveal> {
    let (wf, hf) = (f64::from(w), f64::from(h));
    let dir = tr.direction;
    let cell_x = tr.xsize.max(1);
    let cell_y = if tr.ysize > 0 { tr.ysize } else { cell_x };
    let strip = if tr.xsize > 0 {
        tr.xsize
    } else if tr.ysize > 0 {
        tr.ysize
    } else {
        16
    };
    // Centre point for centred shapes: 0 centre, 1-4 corners.
    let centre = match dir {
        1 => (0.0, 0.0),
        2 => (wf, 0.0),
        3 => (0.0, hf),
        4 => (wf, hf),
        _ => (wf / 2.0, hf / 2.0),
    };
    let style = tr.style;
    let boxed: Reveal = match style {
        // Fades: every pixel together (the easing of 50 and 54 is applied
        // to `t` by the caller).
        0 | 50 | 54 => Box::new(|_, _| (0.5, 1.0)),
        1 => Box::new(|_, _| (0.0, 0.0)),
        2 => Box::new(|_, _| (1.0, 0.0)),
        // Dithered fades: ordered-dither thresholds over cells.
        4 | 5 => {
            let steps = 1.0 + f64::from(tr.interpolation.max(0));
            let diagonal = style == 5;
            let cx = if tr.xsize > 0 { tr.xsize } else { 1 };
            let cy = if tr.ysize > 0 { tr.ysize } else { 1 };
            Box::new(move |x, y| {
                let (mut i, mut j) = ((x / cx) as usize, (y / cy) as usize);
                if diagonal {
                    let (a, b) = (i + j, i + 3 * j);
                    i = a;
                    j = b;
                }
                let threshold = f64::from(BAYER4[j % 4][i % 4]) / 16.0;
                (threshold * (1.0 - 1.0 / 16.0) + 0.5 / 16.0, steps / 16.0)
            })
        }
        // Wipes.
        10 | 280..=285 | 290..=295 => Box::new(move |x, y| match dir % 4 {
            0 => sweep(f64::from(y), hf, soft),
            1 => sweep(hf - f64::from(y) - 1.0, hf, soft),
            2 => sweep(f64::from(x), wf, soft),
            _ => sweep(wf - f64::from(x) - 1.0, wf, soft),
        }),
        // Square / diamond / plus shapes shrinking inwards.
        25 | 27 | 45 | 140 => {
            let shape = style;
            let (cx, cy) = centre;
            let reach = match shape {
                25 => wf.max(hf) / 2.0,
                27 => (wf + hf) / 2.0,
                _ => wf.max(hf) / 2.0,
            };
            let reach = if dir == 0 { reach } else { reach * 2.0 };
            Box::new(move |x, y| {
                let (dx, dy) = (
                    (f64::from(x) + 0.5 - cx).abs(),
                    (f64::from(y) + 0.5 - cy).abs(),
                );
                let d = match shape {
                    25 => dx.max(dy),
                    27 => dx + dy,
                    // Plus: distance to the nearest arm.
                    _ => dx.min(dy),
                };
                // Inwards: the outside first.
                sweep(reach - d, reach, soft)
            })
        }
        // Rows / alternate columns wiping inwards.
        30 | 31 | 100 => {
            let vertical_strips = style == 31 || dir >= 2;
            let alternate = style == 31;
            Box::new(move |x, y| {
                let (pos, len, index) = if vertical_strips {
                    (f64::from(y), hf, x / strip)
                } else {
                    (f64::from(x), wf, y / strip)
                };
                let reversed = (alternate && index % 2 == 1) || dir % 2 == 1;
                let p = if reversed { len - pos - 1.0 } else { pos };
                sweep(p, len, soft)
            })
        }
        // Blocks wiping in alternate directions / brick pattern.
        101 | 102 => {
            let brick = style == 102;
            Box::new(move |x, y| {
                let row = y / cell_y;
                let shift = if brick && row % 2 == 1 { cell_x / 2 } else { 0 };
                let column = (x + shift) / cell_x;
                let local = f64::from((x + shift) % cell_x);
                let reversed = (row + column) % 2 == 1;
                let p = if reversed {
                    f64::from(cell_x) - local - 1.0
                } else {
                    local
                };
                sweep(p, f64::from(cell_x), soft)
            })
        }
        // Wipe inwards / outwards (from both edges).
        110 | 111 => {
            let outwards = style == 111;
            Box::new(move |x, y| {
                let (pos, len) = if dir >= 2 {
                    (f64::from(x), wf)
                } else {
                    (f64::from(y), hf)
                };
                let from_edge = pos.min(len - pos - 1.0);
                let half = len / 2.0;
                let p = if outwards {
                    half - from_edge
                } else {
                    from_edge
                };
                sweep(p, half, soft)
            })
        }
        // Venetian blinds.
        120 => Box::new(move |x, y| {
            let (pos, reversed) = match dir % 4 {
                0 => (y % strip, false),
                1 => (y % strip, true),
                2 => (x % strip, false),
                _ => (x % strip, true),
            };
            let p = if reversed { strip - pos - 1 } else { pos };
            sweep(f64::from(p), f64::from(strip), soft.min(f64::from(strip)))
        }),
        // Diagonal wipe.
        130 => Box::new(move |x, y| {
            let (fx, fy) = (f64::from(x), f64::from(y));
            let d = match dir % 4 {
                0 => fx + fy,
                1 => fx + (hf - fy),
                2 => (wf - fx) + fy,
                _ => (wf - fx) + (hf - fy),
            };
            sweep(d, wf + hf, soft)
        }),
        // Rotating plus: angular sweep around the centre, four arms.
        150 => {
            let (cx, cy) = centre;
            Box::new(move |x, y| {
                let angle = (f64::from(y) + 0.5 - cy).atan2(f64::from(x) + 0.5 - cx);
                let quarter = std::f64::consts::FRAC_PI_2;
                let phase = angle.rem_euclid(quarter) / quarter;
                (phase * 0.95 + 0.025, 0.05)
            })
        }
        // Square fades: spiral, linear, rotating, wiping; wiping diamonds.
        61..=65 => {
            let (cx, cy) = (cell_x.max(8), cell_y.max(8));
            let columns = (w + cx - 1) / cx;
            let rows = (h + cy - 1) / cy;
            let total = f64::from(columns * rows).max(1.0);
            let order = spiral_or_linear(style, columns, rows);
            Box::new(move |x, y| {
                let (i, j) = (x / cx, y / cy);
                let rank = order(i, j);
                let start = f64::from(rank) / total * 0.75;
                let local = match style {
                    // Rotating: fade by angle inside the cell.
                    63 => {
                        let a = (f64::from(y % cy) - f64::from(cy) / 2.0)
                            .atan2(f64::from(x % cx) - f64::from(cx) / 2.0);
                        (a + std::f64::consts::PI) / std::f64::consts::TAU
                    }
                    // Wiping inside the cell.
                    64 => f64::from(x % cx) / f64::from(cx),
                    // Diamonds.
                    65 => {
                        let dx = (f64::from(x % cx) - f64::from(cx) / 2.0).abs() / f64::from(cx);
                        let dy = (f64::from(y % cy) - f64::from(cy) / 2.0).abs() / f64::from(cy);
                        dx + dy
                    }
                    _ => 0.5,
                };
                (
                    start + local * 0.25,
                    if style == 61 || style == 62 {
                        0.25
                    } else {
                        0.05
                    },
                )
            })
        }
        // Random block / row fades.
        180 | 181 => {
            let rows = style == 181;
            Box::new(move |x, y| {
                let key = if rows {
                    i64::from(if dir >= 2 { x / strip } else { y / strip })
                } else {
                    i64::from(y / cell_y) * 10_007 + i64::from(x / cell_x)
                };
                (noise(key) * 0.8 + 0.1, 0.2)
            })
        }
        // Columns wiping randomly.
        200..=204 => {
            let variant = style - 200;
            Box::new(move |x, y| {
                let (along, len, index) = if dir >= 2 {
                    (f64::from(x), wf, y / strip)
                } else {
                    (f64::from(y), hf, x / strip)
                };
                let delay = noise(i64::from(index)) * 0.5;
                let p = match variant {
                    // Inwards: from both ends; outwards: from the middle.
                    2 => along.min(len - along - 1.0) / (len / 2.0),
                    3 => 1.0 - along.min(len - along - 1.0) / (len / 2.0),
                    4 => {
                        if index % 2 == 0 {
                            along.min(len - along - 1.0) / (len / 2.0)
                        } else {
                            1.0 - along.min(len - along - 1.0) / (len / 2.0)
                        }
                    }
                    1 if index % 2 == 1 => 1.0 - along / len,
                    _ => along / len,
                };
                (delay + p * 0.5, 0.02)
            })
        }
        // Mask wipes: the new picture appears in order of brightness of a
        // procedural gradient (no mask file is referenced by #SEL).
        900..=903 => Box::new(move |x, y| {
            let v = (f64::from(x) / wf + f64::from(y) / hf) / 2.0;
            let v = if style % 2 == 1 { 1.0 - v } else { v };
            sweep(v * 100.0, 100.0, soft / 4.0)
        }),
        _ => return None,
    };
    Some(boxed)
}

/// Rank of cell `(i, j)` for the square fades.
fn spiral_or_linear(style: i32, columns: i32, rows: i32) -> impl Fn(i32, i32) -> i32 {
    let mut ranks = vec![0i32; (columns * rows).max(1) as usize];
    if style == 61 {
        // Spiral from the outside in.
        let (mut top, mut left, mut bottom, mut right) = (0, 0, rows - 1, columns - 1);
        let mut rank = 0;
        while top <= bottom && left <= right {
            for i in left..=right {
                ranks[(top * columns + i) as usize] = rank;
                rank += 1;
            }
            for j in top + 1..=bottom {
                ranks[(j * columns + right) as usize] = rank;
                rank += 1;
            }
            if top < bottom {
                for i in (left..right).rev() {
                    ranks[(bottom * columns + i) as usize] = rank;
                    rank += 1;
                }
            }
            if left < right {
                for j in (top + 1..bottom).rev() {
                    ranks[(j * columns + left) as usize] = rank;
                    rank += 1;
                }
            }
            top += 1;
            left += 1;
            bottom -= 1;
            right -= 1;
        }
    } else {
        for (index, rank) in ranks.iter_mut().enumerate() {
            let (i, j) = (index as i32 % columns, index as i32 / columns);
            *rank = if style == 62 { j * columns + i } else { i + j };
        }
        if style != 62 {
            // Diagonal ranks are not unique; scale to the cell count.
            let max = (columns + rows - 2).max(1);
            for rank in &mut ranks {
                *rank = *rank * (columns * rows - 1) / max;
            }
        }
    }
    move |i, j| {
        ranks
            .get((j.clamp(0, rows - 1) * columns + i.clamp(0, columns - 1)) as usize)
            .copied()
            .unwrap_or(0)
    }
}

/// A rule image for a reveal style: RGBA (grayscale in RGB, opaque), bright
/// pixels appearing first, plus the soft-edge width in progress units.
/// `None` when the style is not a reveal.
pub fn reveal_rule(values: &[i32], width: u32, height: u32) -> Option<(Vec<u8>, f64)> {
    let tr = Transition::from_sel(values);
    let (w, h) = (i32::try_from(width).ok()?, i32::try_from(height).ok()?);
    if w <= 0 || h <= 0 || i64::from(w) * i64::from(h) > 16 << 20 {
        return None;
    }
    let soft = if tr.interpolation > 0 {
        (2f64.powi(tr.interpolation.min(8)) * 2.5).max(1.0)
    } else {
        1.0
    };
    let reveal = reveal_function(&tr, w, h, soft)?;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    let mut edge = 0f64;
    for y in 0..h {
        for x in 0..w {
            let (at, e) = reveal(x, y);
            edge = edge.max(e);
            let v = ((1.0 - at.clamp(0.0, 1.0)) * 255.0).round() as u8;
            rgba.extend_from_slice(&[v, v, v, 255]);
        }
    }
    Some((rgba, edge))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(style: i32, direction: i32) -> Vec<i32> {
        let mut v = vec![0, 0, 639, 479, 0, 0, 1000, style, direction, 0, 0, 0, 0, 0, 255, 0];
        v.truncate(16);
        v
    }

    #[test]
    fn wipe_reveals_from_its_start_edge() {
        let (rule, _) = reveal_rule(&sel(10, 0), 4, 8).unwrap();
        // Top-to-bottom: the top row is brighter (earlier) than the bottom.
        assert!(rule[0] > rule[(7 * 4) * 4]);
    }

    #[test]
    fn crossfade_is_uniform_and_motion_styles_are_not_reveals() {
        let (rule, _) = reveal_rule(&sel(0, 0), 3, 3).unwrap();
        assert!(rule.chunks(4).all(|p| p == &rule[..4]));
        // Style 20 slides the new picture on: a motion, not a reveal.
        assert!(reveal_rule(&sel(20, 0), 3, 3).is_none());
    }
}
