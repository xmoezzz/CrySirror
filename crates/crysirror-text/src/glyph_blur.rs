// Portions adapted from Kirikiri Z (krkrz/krkrz fd5c4baa6a2ef5978db1bd043634351f48667daf:
// visual/CharacterData.cpp tTVPCharacterData::Blur, visual/glgen/gengl.pl
// TVPChBlurCopy_c, TVPChBlurAddMulCopy_c, TVPChBlurMulCopy_c, fast_int_hypot).
// Copyright (c) W.Dee and contributors; Kirikiri Z Project Contributors.
// Kirikiri licence (docs/third-party/krkrz.txt). Modifications CrySirror
// contributors (MPL-2.0).
//! Blurred glyph coverage for text shadows/edges: a circular kernel weighted
//! `width - distance + 1`, normalised to 2^18 and scaled by `level/256`, with
//! each accumulation clamped to 255. Operates on 8-bit coverage, as the
//! native 256-level path does.

/// Approximate hypotenuse used by the native kernel (integer, not Euclidean).
fn fast_int_hypot(lx: i32, ly: i32) -> i32 {
    let (lx, ly) = (lx.abs(), ly.abs());
    let (len1, len2) = if lx >= ly { (lx, ly) } else { (ly, lx) };
    let t = len2 + (len2 >> 1);
    len1 - (len1 >> 5) - (len1 >> 7) + (t >> 2) + (t >> 6)
}

/// Blur `src` (`width`×`height` 8-bit coverage). With `radius == 0` the
/// coverage is only scaled by `level`. Returns the new cell size; the cell
/// grows by `radius` on every side (its origin moves by `-radius`).
pub fn blur_coverage(src: &[u8], width: usize, height: usize, radius: u32, level: i32) -> (Vec<u8>, usize, usize) {
    debug_assert_eq!(src.len(), width * height);
    if radius == 0 {
        if level == 255 {
            return (src.to_vec(), width, height);
        }
        // TVPChBlurMulCopy: level<<10, then >>18 (i.e. *level/256).
        let l = level << 10;
        let out = src.iter().map(|&v| ((i32::from(v) * l) >> 18).min(255) as u8).collect();
        return (out, width, height);
    }
    let bw = radius as i32;
    let (nw, nh) = (width + 2 * radius as usize, height + 2 * radius as usize);
    let mut dest = vec![0u8; nw * nh];
    let mut lvsum = 0i32;
    for y in -bw..=bw {
        for x in -bw..=bw {
            let len = fast_int_hypot(x, y);
            if len <= bw {
                lvsum += bw - len + 1;
            }
        }
    }
    let lvsum = if lvsum != 0 { (1 << 18) / lvsum } else { 1 << 18 };
    for y in -bw..=bw {
        for x in -bw..=bw {
            let len = fast_int_hypot(x, y);
            if len > bw {
                continue;
            }
            let weight = (i64::from(bw - len + 1) * i64::from(lvsum) * i64::from(level)) >> 8;
            for sy in 0..height {
                let row = (y + bw) as usize + sy;
                let base = row * nw + (x + bw) as usize;
                for (i, &s) in src[sy * width..(sy + 1) * width].iter().enumerate() {
                    let d = &mut dest[base + i];
                    let a = i64::from(*d) + ((i64::from(s) * weight) >> 18);
                    *d = a.min(255) as u8;
                }
            }
        }
    }
    (dest, nw, nh)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blur_spreads_and_level_amplifies() {
        let src = [255u8];
        let (out, w, h) = blur_coverage(&src, 1, 1, 2, 255);
        assert_eq!((w, h), (5, 5));
        // Centre carries the largest weight. The native approximate hypot
        // puts (2, 2) at distance 2, so even the corner is inside radius 2.
        assert!(out[12] > out[11] && out[11] > out[0] && out[0] > 0);
        assert_eq!(fast_int_hypot(2, 2), 2);
        let (strong, ..) = blur_coverage(&src, 1, 1, 2, 2048);
        assert!(strong[11] > out[11]);
        assert_eq!(blur_coverage(&src, 1, 1, 0, 128).0, vec![127]);
    }
}
