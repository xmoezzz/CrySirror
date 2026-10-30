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
    /// Copy all four channels only when the source alpha is nonzero. Alpha
    /// selects coverage; it does not become an additional blend weight.
    CopyNonTransparent,
    /// Conventional straight RGBA source-over, including destination alpha.
    SourceOver,
    /// Encoded-byte scalar interpolation /256; full opacity copies RGBA,
    /// otherwise alpha is zero. No color-space conversion is performed.
    OpaqueScalar256,
    /// Scalar source-alpha /256 interpolation into an opaque bitmap. Writes
    /// alpha zero even at source alpha zero; opacity zero skips the whole draw.
    MaskScalar256,
    /// MaskScalar256's RGB interpolation that keeps the destination alpha
    /// (krkr `TVPAlphaBlend_HDA`: sopa = s.a, or s.a*opacity>>8 below 255).
    MaskHoldAlphaScalar256,
    /// Photoshop-like blend (krkr `tvpps.c`): per channel `f(s, d)`, then
    /// d + ((f - d)·a >> 8) with a = s.a (·opacity >> 8 below 255). With
    /// `hold_alpha` the destination alpha is kept (`_HDA`), otherwise the
    /// result alpha is zero, as the native kernels write it.
    Ps { blend: PsBlend, hold_alpha: bool },
    /// Encoded-byte scalar opaque-source blend onto straight alpha. Full
    /// opacity copies RGB and writes alpha 255; partial opacity uses the
    /// scalar opacity-on-opacity lookup table, ignoring source alpha.
    OpaqueOnAlphaScalar256,
    /// Encoded-byte scalar straight-alpha blend using the scalar lookup table.
    /// This deliberately preserves the scalar transparent-pixel quirks; it is
    /// not the SIMD or conventional /255 source-over profile.
    AlphaOnAlphaScalar256,
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
// Reference contract: krkrz fd5c4baa6a2ef5978db1bd043634351f48667daf,
// visual/tvpgl.c TVPCreateTable and the scalar *_c blend functions. In particular
// TVPCreateTable mixes binary64 constants and binary32 stored intermediates.
// Keep those rounding boundaries rather than substituting an idealized ratio.
// SIMD upstream paths have different alpha/endpoint semantics and are excluded.
fn scalar_alpha_tables() -> &'static [[u8; 2]] {
    static TABLES: std::sync::OnceLock<Vec<[u8; 2]>> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| {
        (0..65536u32)
            .map(|index| {
                let source = index >> 8;
                let dest = index & 255;
                let weight = if dest == 0 {
                    255
                } else {
                    let da = (f64::from(dest) / 255.0) as f32;
                    let sa = (f64::from(source) / 255.0) as f32;
                    let ratio = sa / da;
                    let denominator = (1.0 - f64::from(sa) + f64::from(ratio)) as f32;
                    ((ratio / denominator * 255.0) as u32).min(255) as u8
                };
                let alpha = 255 - (255 - dest) * (255 - source) / 255;
                [weight, alpha as u8]
            })
            .collect()
    })
}
fn lerp256(d: u8, s: u8, weight: u8) -> u8 {
    (i32::from(d) + ((i32::from(s) - i32::from(d)) * i32::from(weight)).div_euclid(256)) as u8
}
/// The krkr Photoshop-like colour operations with exact table forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PsBlend {
    Normal,
    Overlay,
    HardLight,
    SoftLight,
    ColorDodge,
    ColorBurn,
    Lighten,
    Darken,
}

/// `TVPPsMakeTable` (krkrz visual/glgen/tvpps.c): table[s][d].
fn ps_tables() -> &'static [[[u8; 256]; 256]; 4] {
    static TABLES: std::sync::OnceLock<Box<[[[u8; 256]; 256]; 4]>> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| {
        let mut t = Box::new([[[0u8; 256]; 256]; 4]);
        for s in 0..256usize {
            for d in 0..256usize {
                let (sf, df) = (s as f64, d as f64);
                t[0][s][d] = if s >= 128 {
                    ((df / 255.0).powf(128.0 / sf) * 255.0) as u8
                } else {
                    ((df / 255.0).powf((1.0 - sf / 255.0) / 0.5) * 255.0) as u8
                };
                t[1][s][d] = if 255 - s <= d { 0xff } else { (d * 255 / (255 - s)) as u8 };
                t[2][s][d] = if s <= 255 - d { 0 } else { (255 - (255 - d) * 255 / s) as u8 };
                t[3][s][d] = if d < 128 {
                    (s * d * 2 / 255) as u8
                } else {
                    ((s + d) * 2 - s * d * 2 / 255 - 255) as u8
                };
            }
        }
        t
    })
}

fn ps_channel(blend: PsBlend, s: u8, d: u8) -> u8 {
    let [soft, dodge, burn, overlay] = ps_tables();
    let (si, di) = (usize::from(s), usize::from(d));
    match blend {
        PsBlend::Normal => s,
        PsBlend::Overlay => overlay[si][di],
        PsBlend::HardLight => overlay[di][si],
        PsBlend::SoftLight => soft[si][di],
        PsBlend::ColorDodge => dodge[si][di],
        PsBlend::ColorBurn => burn[si][di],
        PsBlend::Lighten => s.max(d),
        PsBlend::Darken => s.min(d),
    }
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
        PixelMath::Ps { blend, hold_alpha } => {
            let a = if opacity == 255 { u32::from(s[3]) } else { u32::from(s[3]) * op >> 8 } as i32;
            for c in 0..3 {
                let f = ps_channel(blend, s[c], d[c]);
                d[c] = (i32::from(d[c]) + ((i32::from(f) - i32::from(d[c])) * a).div_euclid(256)) as u8;
            }
            if !hold_alpha {
                d[3] = 0;
            }
        }
        PixelMath::MaskHoldAlphaScalar256 => {
            let a = if opacity == 255 { u32::from(s[3]) } else { u32::from(s[3]) * op >> 8 } as i32;
            for c in 0..3 {
                d[c] = (i32::from(d[c]) + ((i32::from(s[c]) - i32::from(d[c])) * a).div_euclid(256))
                    as u8;
            }
        }
        PixelMath::OpaqueScalar256 | PixelMath::MaskScalar256 => {
            if mode == PixelMath::OpaqueScalar256 && opacity == 255 {
                d.copy_from_slice(s);
                return;
            }
            let a = if mode == PixelMath::OpaqueScalar256 {
                op
            } else if opacity == 255 {
                u32::from(s[3])
            } else {
                u32::from(s[3]) * op / 256
            } as i32;
            for c in 0..3 {
                d[c] = (i32::from(d[c]) + ((i32::from(s[c]) - i32::from(d[c])) * a).div_euclid(256))
                    as u8;
            }
            d[3] = 0;
        }
        PixelMath::OpaqueOnAlphaScalar256 | PixelMath::AlphaOnAlphaScalar256 => {
            if mode == PixelMath::OpaqueOnAlphaScalar256 && opacity == 255 {
                d[..3].copy_from_slice(&s[..3]);
                d[3] = 255;
                return;
            }
            let alpha = if mode == PixelMath::OpaqueOnAlphaScalar256 {
                op
            } else if opacity == 255 {
                u32::from(s[3])
            } else {
                u32::from(s[3]) * op / 256
            };
            let [weight, alpha] = scalar_alpha_tables()[(alpha as usize) * 256 + usize::from(d[3])];
            for c in 0..3 {
                d[c] = lerp256(d[c], s[c], weight);
            }
            d[3] = alpha;
        }
        PixelMath::Copy | PixelMath::CopyRgb | PixelMath::CopyNonTransparent => {
            if mode == PixelMath::CopyNonTransparent && s[3] == 0 {
                return;
            }
            let channels = if mode == PixelMath::CopyRgb { 3 } else { 4 };
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
    /// Replace alpha with one channel of an equally sized engine surface.
    /// Both inputs are validated before any destination pixel is changed.
    pub fn apply_channel_mask(&mut self, mask: &ImageSurface, channel: usize) -> Result<()> {
        self.validate()?;
        mask.validate()?;
        ensure!(
            channel < 4 && self.width == mask.width && self.height == mask.height,
            "channel mask dimensions/channel mismatch"
        );
        for (dst, src) in self.rgba.chunks_exact_mut(4).zip(mask.rgba.chunks_exact(4)) {
            dst[3] = src[channel];
        }
        Ok(())
    }
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
        blit_rgba8(
            &mut self.rgba,
            [self.width, self.height],
            &src.rgba,
            [src.width, src.height],
            options,
        )
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
    const SCALAR_MODES: [PixelMath; 4] = [
        PixelMath::OpaqueScalar256,
        PixelMath::MaskScalar256,
        PixelMath::OpaqueOnAlphaScalar256,
        PixelMath::AlphaOnAlphaScalar256,
    ];
    // Derived vectors from the unmodified pinned glgen/gengl.pl-generated C,
    // compiled with clang -O2 -fno-fast-math -ffp-contract=off on arm64 macOS.
    // Inputs are self-authored; no upstream implementation or assets are bundled.
    #[test]
    fn scalar_profiles_match_independent_c_endpoint_vectors() {
        let vectors: &[(usize, u8, u8, u8, [u8; 4])] = &[
            (0, 0, 0, 255, [255, 0, 231, 0]),
            (0, 17, 0, 255, [238, 0, 231, 0]),
            (0, 0, 255, 255, [255, 255, 231, 255]),
            (0, 128, 128, 128, [127, 127, 124, 0]),
            (0, 255, 255, 128, [127, 127, 124, 0]),
            (0, 17, 255, 255, [238, 255, 231, 255]),
            (0, 128, 0, 128, [127, 127, 124, 0]),
            (0, 128, 128, 0, [128, 127, 17, 128]),
            (1, 0, 0, 255, [0, 255, 17, 0]),
            (1, 17, 0, 255, [17, 255, 17, 0]),
            (1, 0, 255, 255, [254, 254, 230, 0]),
            (1, 128, 128, 128, [127, 127, 70, 0]),
            (1, 255, 255, 128, [128, 126, 123, 0]),
            (1, 17, 255, 255, [237, 254, 230, 0]),
            (1, 128, 0, 128, [128, 255, 17, 0]),
            (1, 128, 128, 0, [128, 127, 17, 128]),
            (2, 0, 0, 255, [255, 0, 231, 255]),
            (2, 17, 0, 255, [238, 0, 231, 255]),
            (2, 0, 255, 255, [255, 255, 231, 255]),
            (2, 128, 128, 128, [127, 127, 159, 192]),
            (2, 255, 255, 128, [127, 127, 124, 255]),
            (2, 17, 255, 255, [238, 255, 231, 255]),
            (2, 128, 0, 128, [127, 85, 159, 192]),
            (2, 128, 128, 0, [128, 127, 17, 128]),
            (3, 0, 0, 255, [254, 0, 230, 0]),
            (3, 17, 0, 255, [17, 255, 17, 17]),
            (3, 0, 255, 255, [254, 254, 230, 255]),
            (3, 128, 128, 128, [127, 127, 102, 160]),
            (3, 255, 255, 128, [128, 126, 123, 255]),
            (3, 17, 255, 255, [237, 254, 230, 255]),
            (3, 128, 0, 128, [128, 255, 17, 128]),
            (3, 128, 128, 0, [128, 127, 17, 128]),
        ];
        for &(mode, da, sa, opacity, expected) in vectors {
            let mut dest = [da, 255 - sa, 17, da];
            combine(
                &mut dest,
                &[255 - da, sa, 231, sa],
                opacity,
                SCALAR_MODES[mode],
            );
            assert_eq!(
                dest, expected,
                "mode={mode} da={da} sa={sa} opacity={opacity}"
            );
        }
    }
    #[test]
    fn ps_soft_light_matches_the_native_table_and_holds_alpha() {
        // TVPPsTableSoftLight[200][100] = (100/255)^(128/200)*255 truncated = 140.
        assert_eq!(ps_channel(PsBlend::SoftLight, 200, 100), 140);
        assert_eq!(ps_channel(PsBlend::Overlay, 64, 64), 32);
        assert_eq!(ps_channel(PsBlend::ColorDodge, 255, 1), 255);
        let mut d = [100, 100, 100, 77];
        combine(&mut d, &[200, 200, 200, 255], 255, PixelMath::Ps { blend: PsBlend::SoftLight, hold_alpha: true });
        assert_eq!(d, [139, 139, 139, 77]);
        let mut d = [100, 100, 100, 77];
        combine(&mut d, &[200, 200, 200, 255], 255, PixelMath::Ps { blend: PsBlend::SoftLight, hold_alpha: false });
        assert_eq!(d[3], 0);
    }
    #[test]
    fn hold_alpha_blend_keeps_destination_alpha() {
        // krkrz TVPAlphaBlend_HDA(_o): rgb d + ((s-d)*sopa >> 8), alpha kept.
        let mut d = [100, 200, 0, 77];
        combine(&mut d, &[200, 0, 255, 128], 255, PixelMath::MaskHoldAlphaScalar256);
        assert_eq!(d, [150, 100, 127, 77]);
        let mut d = [100, 200, 0, 77];
        combine(&mut d, &[200, 0, 255, 128], 128, PixelMath::MaskHoldAlphaScalar256);
        assert_eq!(d, [125, 150, 63, 77]);
    }
    #[test]
    fn scalar_alpha_clipping_and_opacity_zero_preserve_other_pixels() {
        let source = ImageSurface::filled(2, 1, [127, 128, 231, 128]).unwrap();
        let mut dest = ImageSurface::filled(3, 1, [128, 127, 17, 128]).unwrap();
        let mut opts = options(
            BitmapRect::new(-1, 0, 4, 1),
            [-1, 0],
            PixelMath::AlphaOnAlphaScalar256,
        );
        opts.opacity = 128;
        dest.blit(&source, opts).unwrap();
        assert_eq!(
            dest.rgba,
            [127, 127, 102, 160, 127, 127, 102, 160, 128, 127, 17, 128]
        );
        let before = dest.clone();
        opts.opacity = 0;
        for mode in SCALAR_MODES {
            opts.mode = mode;
            dest.blit(&source, opts).unwrap();
        }
        assert_eq!(dest, before);
    }
    /// The local oracle is generated independently from the pinned upstream C.
    /// Its header contains both entire tables, followed by four modes times
    /// seven opacities times every source/destination alpha pair, in RGBA order.
    #[test]
    #[ignore = "requires CRYSIRROR_SCALAR_ORACLE from the pinned upstream C oracle"]
    fn scalar_profiles_match_full_external_c_oracle() {
        let data = std::fs::read(std::env::var_os("CRYSIRROR_SCALAR_ORACLE").expect("oracle path"))
            .unwrap();
        assert_eq!(data.len(), 4 + 2 * 65536 + 4 * 7 * 256 * 256 * 4);
        assert_eq!(&data[..4], b"CSO1");
        for (index, &[weight, alpha]) in scalar_alpha_tables().iter().enumerate() {
            assert_eq!(weight, data[4 + index], "weight table index {index}");
            assert_eq!(alpha, data[4 + 65536 + index], "alpha table index {index}");
        }
        let mut offset = 4 + 2 * 65536;
        for mode in SCALAR_MODES {
            for opacity in [0, 1, 2, 127, 128, 254, 255] {
                for da in 0..=255u8 {
                    for sa in 0..=255u8 {
                        let mut dest = [da, 255 - sa, 17, da];
                        combine(&mut dest, &[255 - da, sa, 231, sa], opacity, mode);
                        assert_eq!(
                            dest,
                            data[offset..offset + 4],
                            "{mode:?} da={da} sa={sa} opacity={opacity}"
                        );
                        offset += 4;
                    }
                }
            }
        }
        assert_eq!(offset, data.len());
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
    fn nontransparent_copy_uses_alpha_as_a_selector() {
        let src = ImageSurface {
            width: 2,
            height: 1,
            rgba: vec![99, 88, 77, 0, 90, 80, 70, 1],
        };
        let mut dst = ImageSurface::filled(2, 1, [1, 2, 3, 255]).unwrap();
        dst.blit(
            &src,
            options(
                BitmapRect::new(0, 0, 2, 1),
                [0, 0],
                PixelMath::CopyNonTransparent,
            ),
        )
        .unwrap();
        assert_eq!(dst.rgba, [1, 2, 3, 255, 90, 80, 70, 1]);
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

/// Borrowed, checked RGBA blit. Preserves source/destination coordinates while
/// clipping, and uses the same blend semantics as `ImageSurface::blit`.
/// The borrow contract requires distinct source and destination allocations.
pub fn blit_rgba8(
    destination: &mut [u8],
    size: [u32; 2],
    source: &[u8],
    source_size: [u32; 2],
    options: BlitOptions,
) -> Result<()> {
    ensure!(
        crate::rgba_len(size[0], size[1])? == destination.len(),
        "destination pixel length mismatch"
    );
    ensure!(
        crate::rgba_len(source_size[0], source_size[1])? == source.len(),
        "source pixel length mismatch"
    );
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
        .min(i64::from(source_size[0]) - sx)
        .min(i64::from(size[0]) - dx);
    let y2 = i64::from(r.height)
        .min(i64::from(source_size[1]) - sy)
        .min(i64::from(size[1]) - dy);
    for y in y1..y2 {
        for x in x1..x2 {
            let s = (((sy + y) * i64::from(source_size[0]) + sx + x) * 4) as usize;
            let d = (((dy + y) * i64::from(size[0]) + dx + x) * 4) as usize;
            let pixel = &source[s..s + 4];
            if options.color_key.is_some_and(|key| pixel[..3] == key) {
                continue;
            }
            combine(
                &mut destination[d..d + 4],
                pixel,
                options.opacity,
                options.mode,
            );
        }
    }
    Ok(())
}
