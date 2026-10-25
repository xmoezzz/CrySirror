// SPDX-License-Identifier: MPL-2.0
//! Checked CPU bitmap primitives. Coordinates are pixels, rectangles are half-open.
//! Native buffer numbers, presentation, and inclusive-corner conversion belong to adapters.
use crate::ImageSurface;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BitmapRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl BitmapRect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelMath {
    /// Copy/opacity-lerp all four channels; /255 with nearest rounding.
    #[default]
    Copy,
    /// Copy/opacity-lerp RGB only, retaining destination alpha.
    CopyRgb,
    /// Conventional straight RGBA source-over, including destination alpha.
    SourceOver,
    /// Straight mask blend: truncate opacity*source_alpha/255, rounded RGB,
    /// alpha = dst_alpha + floor((255-dst_alpha)*effective_alpha/255).
    Mask255,
    /// Premultiplied RGB + byte mask: src + floor((256-mask)*dst/256),
    /// saturating; mask zero skips, destination alpha is retained.
    /// Only opacity=255 is supported; native fade parameters need a separate operation.
    PremultipliedMask256,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sampling {
    #[default]
    Nearest,
}
const fn opaque() -> u8 {
    255
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlitOptions {
    pub source_rect: BitmapRect,
    pub destination: [i32; 2],
    #[serde(default = "opaque")]
    pub opacity: u8,
    #[serde(default)]
    pub mode: PixelMath,
    /// Exact RGB key, compared before blending; keyed pixels do not write any channel.
    #[serde(default)]
    pub color_key: Option<[u8; 3]>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StretchOptions {
    pub source_rect: BitmapRect,
    pub destination: BitmapRect,
    #[serde(default = "opaque")]
    pub opacity: u8,
    #[serde(default)]
    pub mode: PixelMath,
    #[serde(default)]
    pub sampling: Sampling,
    #[serde(default)]
    pub color_key: Option<[u8; 3]>,
}

fn lerp(d: u8, s: u8, a: u32) -> u8 {
    ((u32::from(d) * (255 - a) + u32::from(s) * a + 127) / 255) as u8
}
fn check_mode(mode: PixelMath, opacity: u8) -> Result<()> {
    ensure!(
        mode != PixelMath::PremultipliedMask256 || opacity == 255,
        "premultiplied_mask256 requires opacity 255; native fades are not implemented"
    );
    Ok(())
}
fn combine(d: &mut [u8], s: &[u8], opacity: u8, mode: PixelMath) {
    let op = u32::from(opacity);
    if op == 0 {
        return;
    }
    match mode {
        PixelMath::Copy | PixelMath::CopyRgb => {
            let channels = if mode == PixelMath::Copy { 4 } else { 3 };
            for c in 0..channels {
                d[c] = lerp(d[c], s[c], op);
            }
        }
        PixelMath::Mask255 => {
            let a = op * u32::from(s[3]) / 255;
            for c in 0..3 {
                d[c] = lerp(d[c], s[c], a);
            }
            d[3] = (u32::from(d[3]) + (255 - u32::from(d[3])) * a / 255) as u8;
        }
        PixelMath::PremultipliedMask256 => {
            if s[3] == 0 {
                return;
            }
            for c in 0..3 {
                d[c] = (u32::from(s[c]) + ((256 - u32::from(s[3])) * u32::from(d[c]) >> 8)).min(255)
                    as u8;
            }
        }
        PixelMath::SourceOver => {
            let sa = (u32::from(s[3]) * op + 127) / 255;
            if sa == 0 {
                return;
            }
            let da = u32::from(d[3]);
            let alpha_numerator = sa * 255 + da * (255 - sa);
            for c in 0..3 {
                let numerator = u32::from(s[c]) * sa * 255 + u32::from(d[c]) * da * (255 - sa);
                d[c] = ((numerator + alpha_numerator / 2) / alpha_numerator) as u8;
            }
            d[3] = ((alpha_numerator + 127) / 255) as u8;
        }
    }
}
fn clipped(rect: BitmapRect, width: u32, height: u32) -> (i64, i64, i64, i64) {
    let x = i64::from(rect.x);
    let y = i64::from(rect.y);
    (
        x.max(0),
        y.max(0),
        (x + i64::from(rect.width)).min(i64::from(width)),
        (y + i64::from(rect.height)).min(i64::from(height)),
    )
}
impl ImageSurface {
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Result<Self> {
        let len = crate::rgba_len(width, height)?;
        let mut pixels = vec![0; len];
        for p in pixels.chunks_exact_mut(4) {
            p.copy_from_slice(&rgba);
        }
        Ok(Self {
            width,
            height,
            rgba: pixels,
        })
    }
    /// Replace all channels in the clipped rectangle, including transparent pixels.
    pub fn fill_rect(&mut self, rect: BitmapRect, rgba: [u8; 4]) -> Result<()> {
        self.validate()?;
        let (x1, y1, x2, y2) = clipped(rect, self.width, self.height);
        for y in y1..y2 {
            for x in x1..x2 {
                let at = ((y * i64::from(self.width) + x) * 4) as usize;
                self.rgba[at..at + 4].copy_from_slice(&rgba);
            }
        }
        Ok(())
    }
    /// Clip source and destination while preserving their original pixel correspondence.
    /// For overlapping self-copies, pass a snapshot of the source surface.
    pub fn blit(&mut self, src: &Self, options: BlitOptions) -> Result<()> {
        self.validate()?;
        src.validate()?;
        check_mode(options.mode, options.opacity)?;
        let r = options.source_rect;
        let (dx, dy) = (
            i64::from(options.destination[0]),
            i64::from(options.destination[1]),
        );
        let (sx, sy) = (i64::from(r.x), i64::from(r.y));
        let x1 = 0.max(-sx).max(-dx);
        let y1 = 0.max(-sy).max(-dy);
        let x2 = i64::from(r.width)
            .min(i64::from(src.width) - sx)
            .min(i64::from(self.width) - dx);
        let y2 = i64::from(r.height)
            .min(i64::from(src.height) - sy)
            .min(i64::from(self.height) - dy);
        for y in y1..y2 {
            for x in x1..x2 {
                let s = (((sy + y) * i64::from(src.width) + sx + x) * 4) as usize;
                let d = (((dy + y) * i64::from(self.width) + dx + x) * 4) as usize;
                let pixel = &src.rgba[s..s + 4];
                if options.color_key.is_some_and(|key| pixel[..3] == key) {
                    continue;
                }
                combine(
                    &mut self.rgba[d..d + 4],
                    pixel,
                    options.opacity,
                    options.mode,
                );
            }
        }
        Ok(())
    }
    /// Nearest-neighbor scaling anchored at the untrimmed destination origin.
    /// Sampling outside the source surface leaves destination pixels untouched.
    pub fn stretch_blit(&mut self, src: &Self, options: StretchOptions) -> Result<()> {
        self.validate()?;
        src.validate()?;
        check_mode(options.mode, options.opacity)?;
        let s = options.source_rect;
        let d = options.destination;
        if s.width == 0 || s.height == 0 || d.width == 0 || d.height == 0 {
            return Ok(());
        }
        let (x1, y1, x2, y2) = clipped(d, self.width, self.height);
        for y in y1..y2 {
            let sy = i64::from(s.y)
                + (((y - i64::from(d.y)) as u64 * u64::from(s.height)) / u64::from(d.height))
                    as i64;
            if sy < 0 || sy >= i64::from(src.height) {
                continue;
            }
            for x in x1..x2 {
                let sx = i64::from(s.x)
                    + (((x - i64::from(d.x)) as u64 * u64::from(s.width)) / u64::from(d.width))
                        as i64;
                if sx < 0 || sx >= i64::from(src.width) {
                    continue;
                }
                let at = ((sy * i64::from(src.width) + sx) * 4) as usize;
                let pixel = &src.rgba[at..at + 4];
                if options.color_key.is_some_and(|key| pixel[..3] == key) {
                    continue;
                }
                let target = ((y * i64::from(self.width) + x) * 4) as usize;
                combine(
                    &mut self.rgba[target..target + 4],
                    pixel,
                    options.opacity,
                    options.mode,
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(rect: BitmapRect, destination: [i32; 2], mode: PixelMath) -> BlitOptions {
        BlitOptions {
            source_rect: rect,
            destination,
            opacity: 255,
            mode,
            color_key: None,
        }
    }
    #[test]
    fn clipping_keeps_source_destination_correspondence_and_accepts_extreme_rects() {
        let mut src = ImageSurface::filled(3, 1, [0, 0, 0, 255]).unwrap();
        src.rgba = vec![10, 0, 0, 1, 20, 0, 0, 2, 30, 0, 0, 3];
        let mut dst = ImageSurface::filled(3, 1, [99, 0, 0, 77]).unwrap();
        dst.blit(
            &src,
            options(BitmapRect::new(-1, 0, 4, 1), [-1, 0], PixelMath::Copy),
        )
        .unwrap();
        assert_eq!(dst.rgba, src.rgba);
        dst.fill_rect(
            BitmapRect::new(i32::MAX, i32::MIN, u32::MAX, u32::MAX),
            [0; 4],
        )
        .unwrap();
        assert_eq!(dst.rgba, src.rgba);
    }
    #[test]
    fn math_profiles_preserve_their_distinct_alpha_and_rounding() {
        let src = ImageSurface::filled(1, 1, [64, 64, 64, 128]).unwrap();
        let mut premul = ImageSurface::filled(1, 1, [200, 100, 50, 17]).unwrap();
        premul
            .blit(
                &src,
                options(
                    BitmapRect::new(0, 0, 1, 1),
                    [0, 0],
                    PixelMath::PremultipliedMask256,
                ),
            )
            .unwrap();
        assert_eq!(premul.rgba, [164, 114, 89, 17]);
        let mut straight = ImageSurface::filled(1, 1, [200, 100, 50, 17]).unwrap();
        straight
            .blit(
                &src,
                options(BitmapRect::new(0, 0, 1, 1), [0, 0], PixelMath::Mask255),
            )
            .unwrap();
        assert_eq!(straight.rgba, [132, 82, 57, 136]);
        let mut standard = ImageSurface::filled(1, 1, [0, 0, 255, 0]).unwrap();
        standard
            .blit(
                &src,
                options(BitmapRect::new(0, 0, 1, 1), [0, 0], PixelMath::SourceOver),
            )
            .unwrap();
        assert_eq!(standard.rgba, [64, 64, 64, 128]);
        let mut opts = options(
            BitmapRect::new(0, 0, 1, 1),
            [0, 0],
            PixelMath::PremultipliedMask256,
        );
        opts.opacity = 128;
        let before = premul.clone();
        assert!(premul.blit(&src, opts).is_err());
        assert_eq!(premul, before);
    }
    #[test]
    fn nearest_clipping_preserves_phase_and_color_key_preserves_alpha() {
        let src = ImageSurface {
            width: 3,
            height: 1,
            rgba: vec![10, 0, 0, 1, 20, 0, 0, 2, 30, 0, 0, 3],
        };
        let mut dst = ImageSurface::filled(3, 1, [99, 0, 0, 77]).unwrap();
        dst.stretch_blit(
            &src,
            StretchOptions {
                source_rect: BitmapRect::new(0, 0, 3, 1),
                destination: BitmapRect::new(-1, 0, 4, 1),
                opacity: 255,
                mode: PixelMath::CopyRgb,
                sampling: Sampling::Nearest,
                color_key: Some([20, 0, 0]),
            },
        )
        .unwrap();
        assert_eq!(dst.rgba, vec![10, 0, 0, 77, 99, 0, 0, 77, 30, 0, 0, 77]);
    }
    #[test]
    fn invalid_surfaces_fail_before_mutation() {
        let mut dst = ImageSurface::filled(1, 1, [1, 2, 3, 4]).unwrap();
        let src = ImageSurface {
            width: 1,
            height: 1,
            rgba: vec![],
        };
        let before = dst.clone();
        assert!(dst
            .blit(
                &src,
                options(BitmapRect::new(0, 0, 1, 1), [0, 0], PixelMath::Copy)
            )
            .is_err());
        assert_eq!(dst, before);
        assert!(ImageSurface::filled(0, 1, [0; 4]).is_err());
    }
}
