//! Fixed-point raster operations on packed 4-byte pixels, with the exact
//! integer arithmetic of Ren'Py's `_renpy` software renderer (`module/core.c`,
//! MIT): block-average pixellation, per-channel byte tables and linear
//! multipliers, the three-pass box approximation of a Gaussian blur, alpha
//! from a mapped source channel, the 8.8 fixed-point bilinear scaler, the
//! 16.16 affine sampler that blends into a destination, constant and
//! image-controlled blends, a float colour matrix and a weighted grey map.
//!
//! Buffers are tightly packed rows of `w * 4` bytes. Channels are bytes in
//! memory order; callers that need colour semantics pass tables and
//! coefficients in that order. Bounds are those of the C code; reads the
//! C code makes one pixel past an edge (with zero weight) are clamped.
use anyhow::{Result, ensure};

/// A packed 4-byte-per-pixel image.
pub struct Px<'a> {
    pub data: &'a [u8],
    pub w: usize,
    pub h: usize,
}

pub struct PxMut<'a> {
    pub data: &'a mut [u8],
    pub w: usize,
    pub h: usize,
}

fn check(len: usize, w: usize, h: usize) -> Result<()> {
    ensure!(len == w * h * 4, "packed pixel buffer of {len} bytes is not {w}x{h}x4");
    Ok(())
}

impl Px<'_> {
    fn at(&self, x: usize, y: usize) -> usize {
        (y.min(self.h.saturating_sub(1)) * self.w + x.min(self.w.saturating_sub(1))) * 4
    }
}

/// `pixellate32_core`: averages `avg_w x avg_h` source blocks and writes
/// each average as an `out_w x out_h` destination block. As upstream, the
/// horizontal extent of a written block uses `out_h`.
pub fn pixellate(src: &Px, dst: &mut PxMut, avg_w: usize, avg_h: usize, out_w: usize, out_h: usize) -> Result<()> {
    check(src.data.len(), src.w, src.h)?;
    check(dst.data.len(), dst.w, dst.h)?;
    ensure!(avg_w > 0 && avg_h > 0, "pixellate block size must be positive");
    let vw = src.w.div_ceil(avg_w);
    let vh = src.h.div_ceil(avg_h);
    for y in 0..vh {
        let (sy, dy) = (avg_h * y, out_h * y);
        let sy1 = (sy + avg_h).min(src.h);
        let dy1 = (dy + out_h).min(dst.h);
        for x in 0..vw {
            let (sx, dx) = (avg_w * x, out_w * x);
            let sx1 = (sx + avg_w).min(src.w);
            let dx1 = (dx + out_h).min(dst.w);
            let mut sum = [0u32; 4];
            let mut n = 0u32;
            for j in sy..sy1 {
                for i in sx..sx1 {
                    let p = (j * src.w + i) * 4;
                    for c in 0..4 {
                        sum[c] += u32::from(src.data[p + c]);
                    }
                    n += 1;
                }
            }
            if n == 0 {
                continue;
            }
            let avg = sum.map(|s| (s / n) as u8);
            for j in dy..dy1 {
                for i in dx..dx1 {
                    let p = (j * dst.w + i) * 4;
                    dst.data[p..p + 4].copy_from_slice(&avg);
                }
            }
        }
    }
    Ok(())
}

/// `map32_core`: each byte through its channel's 256-entry table. A
/// channel without a table (`None`) is left unchanged in `dst`.
pub fn channel_map(src: &Px, dst: &mut PxMut, maps: [Option<&[u8; 256]>; 4]) -> Result<()> {
    check(src.data.len(), src.w, src.h)?;
    ensure!(dst.w == src.w && dst.h == src.h, "channel_map surfaces differ in size");
    for (s, d) in src.data.chunks_exact(4).zip(dst.data.chunks_exact_mut(4)) {
        for c in 0..4 {
            if let Some(m) = maps[c] {
                d[c] = m[usize::from(s[c])];
            }
        }
    }
    Ok(())
}

/// `linmap32_core`: `byte * mul >> 8` per channel (1.0 == 256); `None`
/// leaves the channel unchanged.
pub fn channel_linmap(src: &Px, dst: &mut PxMut, muls: [Option<i32>; 4]) -> Result<()> {
    check(src.data.len(), src.w, src.h)?;
    ensure!(dst.w == src.w && dst.h == src.h, "channel_linmap surfaces differ in size");
    for (s, d) in src.data.chunks_exact(4).zip(dst.data.chunks_exact_mut(4)) {
        for c in 0..4 {
            if let Some(m) = muls[c] {
                d[c] = ((i32::from(s[c]) * m) >> 8) as u8;
            }
        }
    }
    Ok(())
}

/// `blur_filters`: box widths for `n` passes approximating `sigma`.
fn blur_filters(sigma: f32, n: i32) -> (i32, i32, i32) {
    let mut wl = (12.0 * sigma * sigma / n as f32 + 1.0).sqrt().floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let m = ((12.0 * sigma * sigma - (n * wl * wl) as f32 - (4 * n * wl) as f32 - (3 * n) as f32)
        / (-4 * wl - 4) as f32)
        .round() as i32;
    (wl, wu, m)
}

/// `linblur32_core`: a one-dimensional box blur of `2 * radius + 1`
/// samples with the edge pixels repeated.
pub fn linear_blur(src: &[u8], dst: &mut [u8], w: usize, h: usize, radius: i32, vertical: bool) -> Result<()> {
    check(src.len(), w, h)?;
    check(dst.len(), w, h)?;
    let (rows, cols) = if vertical { (w, h) } else { (h, w) };
    let idx = |r: usize, c: usize| if vertical { (c * w + r) * 4 } else { (r * w + c) * 4 };
    let radius = radius.max(0) as usize;
    let divisor = (radius * 2 + 1) as i32;
    if cols == 0 {
        return Ok(());
    }
    for r in 0..rows {
        let left: [i32; 4] = std::array::from_fn(|k| i32::from(src[idx(r, 0) + k]));
        let mut sum = left.map(|v| v * radius as i32);
        let mut leader = 0usize;
        let mut trailer = 0usize;
        let px = |c: usize| -> [i32; 4] {
            let p = idx(r, c.min(cols - 1));
            std::array::from_fn(|k| i32::from(src[p + k]))
        };
        for _ in 0..radius {
            let p = px(leader);
            for k in 0..4 {
                sum[k] += p[k];
            }
            leader += 1;
        }
        let mut c = 0usize;
        let mut out = |c: usize, sum: &[i32; 4], dst: &mut [u8]| {
            let p = idx(r, c);
            for k in 0..4 {
                dst[p + k] = (sum[k] / divisor) as u8;
            }
        };
        while c < radius && c < cols {
            let p = px(leader);
            for k in 0..4 {
                sum[k] += p[k];
            }
            leader += 1;
            out(c, &sum, dst);
            for k in 0..4 {
                sum[k] -= left[k];
            }
            c += 1;
        }
        let end = cols as isize - radius as isize - 1;
        while (c as isize) < end {
            let p = px(leader);
            for k in 0..4 {
                sum[k] += p[k];
            }
            leader += 1;
            out(c, &sum, dst);
            let t = px(trailer);
            for k in 0..4 {
                sum[k] -= t[k];
            }
            trailer += 1;
            c += 1;
        }
        let right = px(leader);
        while c < cols {
            for k in 0..4 {
                sum[k] += right[k];
            }
            out(c, &sum, dst);
            let t = px(trailer);
            for k in 0..4 {
                sum[k] -= t[k];
            }
            trailer += 1;
            c += 1;
        }
    }
    Ok(())
}

/// `blur32_core`: three horizontal + vertical box passes approximating a
/// Gaussian of the given radii; `work` holds intermediate rows.
pub fn gaussian_box_blur(src: &[u8], work: &mut [u8], dst: &mut [u8], w: usize, h: usize, xrad: f32, yrad: f32) -> Result<()> {
    let n = 3;
    let (xl, xu, xm) = blur_filters(xrad, n);
    let (yl, yu, ym) = if xrad != yrad { blur_filters(yrad, n) } else { (xl, xu, xm) };
    let mut cur = src.to_vec();
    for i in 0..n {
        let xr = if i < xm { xl } else { xu };
        linear_blur(&cur, work, w, h, xr, false)?;
        let yr = if i < ym { yl } else { yu };
        linear_blur(work, dst, w, h, yr, true)?;
        cur.copy_from_slice(dst);
    }
    Ok(())
}

/// `alphamunge_core`: `dst[alpha] = amap[src[channel]]` over `dst`'s size.
pub fn alpha_munge(src: &Px, dst: &mut PxMut, src_channel: usize, dst_alpha: usize, amap: &[u8; 256]) -> Result<()> {
    check(src.data.len(), src.w, src.h)?;
    ensure!(src.w >= dst.w && src.h >= dst.h, "alpha_munge source is smaller than the destination");
    for y in 0..dst.h {
        for x in 0..dst.w {
            let s = src.data[(y * src.w + x) * 4 + src_channel];
            dst.data[(y * dst.w + x) * 4 + dst_alpha] = amap[usize::from(s)];
        }
    }
    Ok(())
}

/// Source and destination rectangles of [`bilinear`].
#[derive(Clone, Copy, Debug)]
pub struct ScaleRects {
    pub sx: f32,
    pub sy: f32,
    pub sw: f32,
    pub sh: f32,
    pub dx: f32,
    pub dy: f32,
    pub dw: f32,
    pub dh: f32,
}

/// `scale32_core`: 8.8 fixed-point bilinear resampling of a source
/// rectangle into the whole destination (`precise` selects the
/// endpoint-exact deltas).
pub fn bilinear(src: &Px, dst: &mut PxMut, r: ScaleRects, precise: bool) -> Result<()> {
    check(src.data.len(), src.w, src.h)?;
    check(dst.data.len(), dst.w, dst.h)?;
    let (xdelta, ydelta) = if precise {
        (
            if r.dw > 1.0 { 256.0 * (r.sw - 1.0) / (r.dw - 1.0) } else { 0.0 },
            if r.dh > 1.0 { 256.0 * (r.sh - 1.0) / (r.dh - 1.0) } else { 0.0 },
        )
    } else {
        (255.0 * (r.sw - 1.0) / r.dw, 255.0 * (r.sh - 1.0) / r.dh)
    };
    for y in 0..dst.h {
        let sline = (r.sy * 256.0 + (y as f32 + r.dy) * ydelta) as i32;
        let s1frac = sline & 255;
        let s0frac = 256 - s1frac;
        let row = (sline >> 8).max(0) as usize;
        let mut scol = r.sx * 256.0 + r.dx * xdelta;
        for x in 0..dst.w {
            let xfrac0 = 256 - ((scol as i32) & 255);
            let col = ((scol as i32) >> 8).max(0) as usize;
            let mut acc = [0i32; 4];
            for (k, xf) in [(0usize, xfrac0), (1, 256 - xfrac0)] {
                let p0 = src.at(col + k, row);
                let p1 = src.at(col + k, row + 1);
                for c in 0..4 {
                    let v = (i32::from(src.data[p0 + c]) * s0frac + i32::from(src.data[p1 + c]) * s1frac) >> 8;
                    acc[c] += v * xf;
                }
            }
            let d = (y * dst.w + x) * 4;
            for c in 0..4 {
                dst.data[d + c] = ((acc[c] as u16) >> 8) as u8;
            }
            scol += xdelta;
        }
    }
    Ok(())
}

/// `I(a, b, mul)` on two packed channel pairs.
fn lerp2(a: u32, b: u32, mul: u32) -> u32 {
    ((((b.wrapping_sub(a)).wrapping_mul(mul)) >> 8).wrapping_add(a)) & 0x00ff_00ff
}

/// `transform32_std`: samples the source through the affine map
/// `(corner + x * (xdx, ydx) + y * (xdy, ydy))` with 16.16 bilinear
/// filtering and blends it over `dst` by the sample's channel `ashift`
/// times `alpha`. `precise` is false for pre-6.10.1 layouts without the
/// one-pixel border (an epsilon is applied as upstream).
#[allow(clippy::too_many_arguments)]
pub fn affine_blend(
    src: &Px,
    dst: &mut PxMut,
    corner: [f32; 2],
    mut xdx: f32,
    mut ydx: f32,
    mut xdy: f32,
    mut ydy: f32,
    ashift: u32,
    alpha: f32,
    precise: bool,
) -> Result<()> {
    const EPS: f32 = 1.0 / 256.0;
    check(src.data.len(), src.w, src.h)?;
    check(dst.data.len(), dst.w, dst.h)?;
    let amul = (alpha * 256.0) as u32;
    let mut maxsx = src.w as f64;
    let mut maxsy = src.h as f64;
    if !precise {
        maxsx -= f64::from(EPS);
        maxsy -= f64::from(EPS);
        for d in [&mut xdx, &mut xdy, &mut ydx, &mut ydy] {
            if *d != 0.0 && (1.0 / *d % 1.0).abs() < EPS {
                *d -= (*d / d.abs()) * EPS;
            }
        }
    }
    let word = |p: usize| u32::from_le_bytes(src.data[p..p + 4].try_into().unwrap());
    for y in 0..dst.h {
        let leftsx = f64::from(corner[0]) + y as f64 * f64::from(xdy);
        let leftsy = f64::from(corner[1]) + y as f64 * f64::from(ydy);
        let mut minx = 0.0f64;
        let mut maxx = dst.w as f64 - 1.0;
        let mut skip = false;
        for (left, d, maxs) in [(leftsx, xdx, maxsx), (leftsy, ydx, maxsy)] {
            if d != 0.0 {
                let x1 = (0.0 - left) / f64::from(d);
                let x2 = (maxs - left) / f64::from(d);
                let (lo, hi) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
                minx = minx.max(lo);
                maxx = maxx.min(hi);
            } else if left < 0.0 || left > maxs {
                skip = true;
            }
        }
        if skip {
            continue;
        }
        let minx = minx.ceil();
        let maxx = maxx.floor();
        if minx >= maxx {
            continue;
        }
        let mut sxi = ((leftsx + minx * f64::from(xdx)) * 65536.0) as i32;
        let mut syi = ((leftsy + minx * f64::from(ydx)) * 65536.0) as i32;
        let dsxi = (f64::from(xdx) * 65536.0) as i32;
        let dsyi = (f64::from(ydx) * 65536.0) as i32;
        for x in (minx as usize)..=(maxx as usize) {
            let (px, py) = ((sxi >> 16).max(0) as usize, (syi >> 16).max(0) as usize);
            let yfrac = ((syi >> 8) & 0xff) as u32;
            let xfrac = ((sxi >> 8) & 0xff) as u32;
            let mut pal = word(src.at(px, py));
            let mut pbl = word(src.at(px + 1, py));
            let mut pcl = word(src.at(px, py + 1));
            let mut pdl = word(src.at(px + 1, py + 1));
            let pah = (pal >> 8) & 0x00ff_00ff;
            let pbh = (pbl >> 8) & 0x00ff_00ff;
            let pch = (pcl >> 8) & 0x00ff_00ff;
            let pdh = (pdl >> 8) & 0x00ff_00ff;
            pal &= 0x00ff_00ff;
            pbl &= 0x00ff_00ff;
            pcl &= 0x00ff_00ff;
            pdl &= 0x00ff_00ff;
            let rh = lerp2(lerp2(pah, pch, yfrac), lerp2(pbh, pdh, yfrac), xfrac);
            let rl = lerp2(lerp2(pal, pcl, yfrac), lerp2(pbl, pdl, yfrac), xfrac);
            let a = ((((rh << 8) | rl) >> ashift) & 0xff) * amul >> 8;
            let d = (y * dst.w + x) * 4;
            let mut dl = u32::from_le_bytes(dst.data[d..d + 4].try_into().unwrap());
            let mut dh = (dl >> 8) & 0x00ff_00ff;
            dl &= 0x00ff_00ff;
            dl = lerp2(dl, rl, a);
            dh = lerp2(dh, rh, a);
            dst.data[d..d + 4].copy_from_slice(&((dh << 8) | dl).to_le_bytes());
            sxi = sxi.wrapping_add(dsxi);
            syi = syi.wrapping_add(dsyi);
        }
    }
    Ok(())
}

fn blend_words(a: u32, b: u32, alpha: u32) -> u32 {
    let (ah, bh) = ((a >> 8) & 0x00ff_00ff, (b >> 8) & 0x00ff_00ff);
    lerp2(a & 0x00ff_00ff, b & 0x00ff_00ff, alpha) | (lerp2(ah, bh, alpha) << 8)
}

/// `blend32_core`: `a + (b - a) * alpha / 256` per channel.
pub fn blend(a: &Px, b: &Px, dst: &mut PxMut, alpha: u32) -> Result<()> {
    check(dst.data.len(), dst.w, dst.h)?;
    ensure!(a.w >= dst.w && a.h >= dst.h && b.w >= dst.w && b.h >= dst.h, "blend sources are too small");
    for y in 0..dst.h {
        for x in 0..dst.w {
            let (pa, pb, d) = ((y * a.w + x) * 4, (y * b.w + x) * 4, (y * dst.w + x) * 4);
            let wa = u32::from_le_bytes(a.data[pa..pa + 4].try_into().unwrap());
            let wb = u32::from_le_bytes(b.data[pb..pb + 4].try_into().unwrap());
            dst.data[d..d + 4].copy_from_slice(&blend_words(wa, wb, alpha).to_le_bytes());
        }
    }
    Ok(())
}

/// `imageblend32_core`: [`blend`] with a per-pixel weight
/// `amap[img[alpha_off]]`.
pub fn image_blend(a: &Px, b: &Px, dst: &mut PxMut, img: &Px, alpha_off: usize, amap: &[u8; 256]) -> Result<()> {
    check(dst.data.len(), dst.w, dst.h)?;
    ensure!(
        a.w >= dst.w && a.h >= dst.h && b.w >= dst.w && b.h >= dst.h && img.w >= dst.w && img.h >= dst.h,
        "image_blend sources are too small"
    );
    for y in 0..dst.h {
        for x in 0..dst.w {
            let (pa, pb, pi, d) =
                ((y * a.w + x) * 4, (y * b.w + x) * 4, (y * img.w + x) * 4 + alpha_off, (y * dst.w + x) * 4);
            let alpha = u32::from(amap[usize::from(img.data[pi])]);
            let wa = u32::from_le_bytes(a.data[pa..pa + 4].try_into().unwrap());
            let wb = u32::from_le_bytes(b.data[pb..pb + 4].try_into().unwrap());
            dst.data[d..d + 4].copy_from_slice(&blend_words(wa, wb, alpha).to_le_bytes());
        }
    }
    Ok(())
}

/// `colormatrix32_core`: rows of `[c0, c1, c2, c3, offset]` over the four
/// bytes; the offset is scaled by 255 and truncated as upstream.
pub fn color_matrix(src: &Px, dst: &mut PxMut, m: &[[f32; 5]; 4]) -> Result<()> {
    check(dst.data.len(), dst.w, dst.h)?;
    ensure!(src.w >= dst.w && src.h >= dst.h, "color_matrix source is too small");
    let off: [i32; 4] = std::array::from_fn(|i| (m[i][4] * 255.0) as i32);
    for y in 0..dst.h {
        for x in 0..dst.w {
            let (ps, d) = ((y * src.w + x) * 4, (y * dst.w + x) * 4);
            let s: [f32; 4] = std::array::from_fn(|k| f32::from(src.data[ps + k]));
            for i in 0..4 {
                let r = off[i] + (m[i][0] * s[0] + m[i][1] * s[1] + m[i][2] * s[2] + m[i][3] * s[3]) as i32;
                dst.data[d + i] = r.clamp(0, 255) as u8;
            }
        }
    }
    Ok(())
}

/// `staticgray_core`: `vmap[(sum of byte * mul) >> shift]`, written to
/// one byte per pixel.
pub fn static_gray(src: &Px, dst: &mut [u8], dst_pitch: usize, muls: [i32; 4], shift: u32, vmap: &[u8]) -> Result<()> {
    check(src.data.len(), src.w, src.h)?;
    for y in 0..src.h {
        for x in 0..src.w {
            let p = (y * src.w + x) * 4;
            let sum: i32 = (0..4).map(|k| i32::from(src.data[p + k]) * muls[k]).sum();
            let v = vmap.get((sum >> shift).max(0) as usize).copied().unwrap_or(0);
            dst[y * dst_pitch + x] = v;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixellate_averages_blocks() {
        let src: Vec<u8> = [[0u8, 0, 0, 0], [100, 100, 100, 100], [50, 50, 50, 50], [10, 10, 10, 10]].concat();
        let mut dst = vec![0u8; 16];
        pixellate(&Px { data: &src, w: 2, h: 2 }, &mut PxMut { data: &mut dst, w: 2, h: 2 }, 2, 2, 2, 2).unwrap();
        assert!(dst.iter().all(|v| *v == 40));
    }

    #[test]
    fn blur_keeps_flat_images_and_spreads_points() {
        let w = 9;
        let flat = vec![200u8; w * w * 4];
        let (mut work, mut out) = (vec![0u8; flat.len()], vec![0u8; flat.len()]);
        gaussian_box_blur(&flat, &mut work, &mut out, w, w, 2.0, 2.0).unwrap();
        assert!(out.iter().all(|v| *v == 200));
        let mut point = vec![0u8; w * w * 4];
        let c = (4 * w + 4) * 4;
        point[c..c + 4].copy_from_slice(&[255; 4]);
        gaussian_box_blur(&point, &mut work, &mut out, w, w, 1.0, 1.0).unwrap();
        assert!(out[c] < 255 && out[c] > 0 && out[(4 * w + 5) * 4] > 0);
    }

    #[test]
    fn blend_and_matrix_match_fixed_point() {
        let a = [0u8, 100, 200, 255];
        let b = [255u8, 0, 100, 0];
        let mut d = [0u8; 4];
        blend(&Px { data: &a, w: 1, h: 1 }, &Px { data: &b, w: 1, h: 1 }, &mut PxMut { data: &mut d, w: 1, h: 1 }, 128).unwrap();
        // The packed-pair arithmetic of core.c (checked against the C macro).
        assert_eq!(d, [127, 50, 150, 127]);
        let id = [[1.0, 0.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0, 0.0]];
        color_matrix(&Px { data: &a, w: 1, h: 1 }, &mut PxMut { data: &mut d, w: 1, h: 1 }, &id).unwrap();
        assert_eq!(d, a);
    }

    #[test]
    fn bilinear_identity_scale() {
        let src: Vec<u8> = (0..16u8).flat_map(|i| [i * 10; 4]).collect();
        let mut dst = vec![0u8; src.len()];
        let r = ScaleRects { sx: 0.0, sy: 0.0, sw: 4.0, sh: 4.0, dx: 0.0, dy: 0.0, dw: 4.0, dh: 4.0 };
        bilinear(&Px { data: &src, w: 4, h: 4 }, &mut PxMut { data: &mut dst, w: 4, h: 4 }, r, true).unwrap();
        assert_eq!(dst, src);
    }
}
