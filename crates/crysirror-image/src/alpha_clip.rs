// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Bounded two-input stored-coverage multiplication and optional exterior clear.
use crate::ImageSurface;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

/// Integer image coordinates; the source's own draw clip does not constrain
/// sampling. RGB remains unchanged unless `multiply_rgb` is explicitly set.
/// Products use `(p + (p >> 7)) >> 8`, including all endpoint byte pairs.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlphaClipRect {
    pub destination: [i32; 2],
    /// x,y,width,height; width and height must be positive.
    pub source: [i32; 4],
    /// Destination x,y,width,height, intersected with its image bounds.
    pub clip: [i32; 4],
    /// Set exterior coverage (or multiply exterior RGBA with multiply_rgb).
    pub clear: Option<u8>,
    #[serde(default)]
    pub multiply_rgb: bool,
    /// Read source coverage from the current destination, walking forward.
    /// This deliberately preserves overlapping in-place feedback.
    #[serde(default)]
    pub self_source: bool,
    /// Explicit profile: an entirely disjoint source clears only coverage,
    /// even when intersecting operations would multiply all four channels.
    #[serde(default)]
    pub clear_empty_alpha_only: bool,
}
impl AlphaClipRect {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source[2] > 0 && self.source[3] > 0,
            "alpha clip source extent"
        );
        ensure!(
            self.clip[2] >= 0 && self.clip[3] >= 0,
            "alpha clip destination extent"
        );
        Ok(())
    }
    pub fn apply(
        &self,
        destination: &ImageSurface,
        source: &ImageSurface,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<ImageSurface> {
        self.validate()?;
        destination.validate()?;
        source.validate()?;
        check()?;
        ensure!(
            u64::from(destination.width) * u64::from(destination.height)
                <= crate::effects::MAX_EFFECT_PIXELS,
            "alpha clip pixel budget"
        );
        if self.self_source {
            ensure!(
                [source.width, source.height] == [destination.width, destination.height],
                "self alpha clip dimensions"
            );
        }
        let [dw, dh] = [i64::from(destination.width), i64::from(destination.height)];
        let [sw, sh] = [i64::from(source.width), i64::from(source.height)];
        let [dx, dy] = self.destination.map(i64::from);
        let [sx, sy, w, h] = self.source.map(i64::from);
        let [cx, cy, cw, ch] = self.clip.map(i64::from);
        let [cl, ct, cr, cb] = [cx.max(0), cy.max(0), (cx + cw).min(dw), (cy + ch).min(dh)];
        let [left, top, right, bottom] = [
            0.max(-sx).max(cl - dx),
            0.max(-sy).max(ct - dy),
            w.min(sw - sx).min(cr - dx),
            h.min(sh - sy).min(cb - dy),
        ];
        let mut out = destination.clone();
        if cr <= cl || cb <= ct {
            return Ok(out);
        }
        let overlap = right > left && bottom > top;
        let alpha_only = !self.multiply_rgb || (!overlap && self.clear_empty_alpha_only);
        let clear_pixel = |out: &mut ImageSurface, x: i64, y: i64| {
            if let Some(a) = self.clear {
                let offset = ((y * dw + x) * 4) as usize;
                if alpha_only {
                    out.rgba[offset + 3] = a;
                } else {
                    for c in 0..4 {
                        out.rgba[offset + c] = multiply(out.rgba[offset + c], a);
                    }
                }
            }
        };
        if !overlap {
            if self.clear.is_some() {
                for y in ct..cb {
                    check()?;
                    for x in cl..cr {
                        clear_pixel(&mut out, x, y);
                    }
                }
            }
            return Ok(out);
        }
        // Exterior top/bottom rows precede the band, including self-aliasing.
        if self.clear.is_some() {
            for y in (ct..dy + top).chain(dy + bottom..cb) {
                check()?;
                for x in cl..cr {
                    clear_pixel(&mut out, x, y);
                }
            }
        }
        for y in top..bottom {
            check()?;
            if self.clear.is_some() {
                for x in cl..dx + left {
                    clear_pixel(&mut out, x, dy + y);
                }
            }
            for x in left..right {
                if x % 4096 == 0 {
                    check()?;
                }
                let src = (((sy + y) * sw + sx + x) * 4 + 3) as usize;
                let a = if self.self_source {
                    out.rgba[src]
                } else {
                    source.rgba[src]
                };
                let dst = (((dy + y) * dw + dx + x) * 4) as usize;
                for c in if self.multiply_rgb { 0..4 } else { 3..4 } {
                    out.rgba[dst + c] = multiply(out.rgba[dst + c], a);
                }
            }
            if self.clear.is_some() {
                for x in dx + right..cr {
                    clear_pixel(&mut out, x, dy + y);
                }
            }
        }
        check()?;
        Ok(out)
    }
}
fn multiply(a: u8, b: u8) -> u8 {
    let p = u32::from(a) * u32::from(b);
    ((p + (p >> 7)) >> 8) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> AlphaClipRect {
        AlphaClipRect {
            destination: [0, 0],
            source: [0, 0, 256, 256],
            clip: [0, 0, 256, 256],
            clear: None,
            multiply_rgb: false,
            self_source: false,
            clear_empty_alpha_only: false,
        }
    }
    #[test]
    fn all_coverage_pairs_keep_rgb_and_exact_integer_products() {
        let mut d = ImageSurface::filled(256, 256, [7, 11, 19, 0]).unwrap();
        let mut s = d.clone();
        for y in 0..256 {
            for x in 0..256 {
                let i = (y * 256 + x) * 4;
                d.rgba[i + 3] = y as u8;
                s.rgba[i + 3] = x as u8;
            }
        }
        let out = spec().apply(&d, &s, &|| Ok(())).unwrap();
        for y in 0..256 {
            for x in 0..256 {
                let p = y * x;
                let i = (y * 256 + x) * 4;
                assert_eq!(&out.rgba[i..i + 3], &[7, 11, 19]);
                assert_eq!(out.rgba[i + 3], ((p + (p >> 7)) >> 8) as u8);
            }
        }
    }
    #[test]
    fn forward_self_overlap_and_outside_clear_have_explicit_profiles() {
        let mut d = ImageSurface::filled(4, 1, [9, 13, 17, 0]).unwrap();
        for (i, a) in [255, 128, 64, 32].into_iter().enumerate() {
            d.rgba[i * 4 + 3] = a;
        }
        let mut p = spec();
        p.source = [0, 0, 3, 1];
        p.destination = [1, 0];
        p.clip = [0, 0, 4, 1];
        p.self_source = true;
        let out = p.apply(&d, &d, &|| Ok(())).unwrap();
        assert_eq!(
            out.rgba.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>(),
            [255, 128, 32, 4]
        );
        p.self_source = false;
        p.source = [100, 100, 1, 1];
        p.clear = Some(0);
        p.multiply_rgb = true;
        p.clear_empty_alpha_only = true;
        let out = p.apply(&d, &d, &|| Ok(())).unwrap();
        assert!(out.rgba.chunks_exact(4).all(|p| p == [9, 13, 17, 0]));
        assert!(p
            .apply(&d, &d, &|| anyhow::bail!("authored cancellation"))
            .is_err());
    }
    #[test]
    fn clipping_clear_and_premultiplied_channels_preserve_exact_extents() {
        let d = ImageSurface::filled(4, 3, [4, 8, 16, 128]).unwrap();
        let s = ImageSurface::filled(3, 2, [0, 0, 0, 64]).unwrap();
        let mut p = spec();
        p.destination = [0, 0];
        p.source = [-1, -1, 6, 4];
        p.clip = [1, 1, 2, 1];
        p.clear = Some(0);
        let out = p.apply(&d, &s, &|| Ok(())).unwrap();
        for y in 0..3 {
            for x in 0..4 {
                let i = (y * 4 + x) * 4;
                assert_eq!(&out.rgba[i..i + 3], &[4, 8, 16]);
                assert_eq!(
                    out.rgba[i + 3],
                    if y == 1 && (x == 1 || x == 2) {
                        32
                    } else {
                        128
                    }
                );
            }
        }
        p.destination = [1, 1];
        p.source = [0, 0, 1, 1];
        p.clip = [0, 0, 4, 3];
        p.multiply_rgb = true;
        p.clear = Some(128);
        let out = p.apply(&d, &s, &|| Ok(())).unwrap();
        for y in 0..3 {
            for x in 0..4 {
                let i = (y * 4 + x) * 4;
                assert_eq!(
                    &out.rgba[i..i + 4],
                    if y == 1 && x == 1 {
                        &[1, 2, 4, 32]
                    } else {
                        &[2, 4, 8, 64]
                    }
                );
            }
        }
        p.destination = [i32::MIN, i32::MAX];
        p.source = [i32::MAX, i32::MIN, i32::MAX, i32::MAX];
        p.clear = None;
        assert_eq!(p.apply(&d, &s, &|| Ok(())).unwrap().rgba, d.rgba);
    }
}
