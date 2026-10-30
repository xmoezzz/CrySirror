// SPDX-License-Identifier: MPL-2.0
//! Integer associated-RGB operations with explicit coverage and weight.
//!
//! These operations work on stored channel bytes. They do not convert color
//! spaces or normalize associated RGB into straight-alpha display pixels.
use crate::{bitmap::BitmapRect, ImageSurface};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

/// Stored associated-channel tone adjustment. Channel order is caller-defined;
/// the three percentages address the first three bytes and alpha is byte four.
/// Percent 100 is identity; other values clamp colour to coverage. This kernel
/// preserves the RFVP 0..200 arithmetic without owning native texture IDs.
pub fn tone_channels_percent(pixels: &mut [u8], percent: [i32; 3]) -> Result<()> {
    ensure!(
        pixels.len().is_multiple_of(4),
        "tone requires four-channel pixels"
    );
    ensure!(pixels.len() <= crate::MAX_DECODED_BYTES, "tone byte budget");
    let percent = percent.map(|p| p.clamp(0, 200) as u32);
    if percent == [100; 3] {
        return Ok(());
    }
    for p in pixels.chunks_exact_mut(4) {
        let a = u32::from(p[3]);
        for i in 0..3 {
            let c = u32::from(p[i]);
            let t = percent[i];
            p[i] = (if t < 100 {
                t * c / 100
            } else if t == 100 {
                c
            } else {
                c + a * (t - 100) / 255
            })
            .min(a)
            .min(255) as u8;
        }
    }
    Ok(())
}

/// A checked, tightly packed byte coverage plane, independent of RGBA alpha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveragePlane {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}
impl CoveragePlane {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            crate::rgba_len(self.width, self.height)? / 4 == self.bytes.len(),
            "coverage plane length mismatch"
        );
        Ok(())
    }
    pub fn from_alpha(surface: &ImageSurface) -> Result<Self> {
        surface.validate()?;
        Ok(Self {
            width: surface.width,
            height: surface.height,
            bytes: surface.rgba.chunks_exact(4).map(|p| p[3]).collect(),
        })
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssociatedBlend {
    /// Replace RGB without using coverage, then weight against old RGB.
    Copy,
    /// Saturated source + floor(destination * (256 - coverage) / 256).
    #[default]
    Over,
    /// Saturated destination + floor(source * (256 - destination alpha) / 256).
    Under,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageWrite {
    #[default]
    Preserve,
    Source,
}
fn full_weight() -> u16 {
    256
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssociatedBlitOptions {
    pub source_rect: BitmapRect,
    pub destination: [i32; 2],
    #[serde(default)]
    pub blend: AssociatedBlend,
    /// Fixed-point blend weight 0..=256; 256 applies the full result.
    #[serde(default = "full_weight")]
    pub weight: u16,
    #[serde(default)]
    pub coverage_write: CoverageWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RgbFilter {
    /// Move towards RGB by floor((target - current) * weight / 256).
    Fade { rgb: [u8; 3], weight: u16 },
    /// Multiply each RGB byte by the corresponding factor / 256.
    Multiply { factors: [u16; 3] },
}

impl ImageSurface {
    /// Composite associated RGB with independent source coverage. If omitted,
    /// source RGBA alpha is the coverage byte. Destination alpha is a coverage
    /// payload, preserved unless `coverage_write` explicitly replaces it.
    ///
    /// `Under` weights against the source (the original background), whereas
    /// `Over`/`Copy` weight against the old destination. A zero coverage `Over`
    /// skips RGB even for nonzero stored source channels. For `Under`, zero
    /// destination coverage copies source RGB regardless of weight.
    pub fn composite_associated(
        &mut self,
        source: &Self,
        coverage: Option<&CoveragePlane>,
        options: AssociatedBlitOptions,
    ) -> Result<()> {
        self.validate()?;
        source.validate()?;
        ensure!(
            options.weight <= 256,
            "associated blend weight must be 0..256"
        );
        if let Some(plane) = coverage {
            plane.validate()?;
            ensure!(
                (plane.width, plane.height) == (source.width, source.height),
                "coverage dimensions must match source"
            );
        }
        let rect = options.source_rect;
        let [dx, dy] = options.destination.map(i64::from);
        let (sx, sy) = (i64::from(rect.x), i64::from(rect.y));
        let x0 = 0.max(-sx).max(-dx);
        let y0 = 0.max(-sy).max(-dy);
        let x1 = i64::from(rect.width)
            .min(i64::from(source.width) - sx)
            .min(i64::from(self.width) - dx);
        let y1 = i64::from(rect.height)
            .min(i64::from(source.height) - sy)
            .min(i64::from(self.height) - dy);
        for y in y0..y1 {
            for x in x0..x1 {
                let si = ((sy + y) * i64::from(source.width) + sx + x) as usize;
                let di = (((dy + y) * i64::from(self.width) + dx + x) * 4) as usize;
                let src = &source.rgba[si * 4..si * 4 + 4];
                let mask = coverage.map_or(src[3], |plane| plane.bytes[si]);
                let dst = &mut self.rgba[di..di + 4];
                let weight = u32::from(options.weight);
                for channel in 0..3 {
                    let s = u32::from(src[channel]);
                    let d = u32::from(dst[channel]);
                    let (composed, base) = match options.blend {
                        AssociatedBlend::Copy => (s, d),
                        AssociatedBlend::Over if mask == 0 => (d, d),
                        AssociatedBlend::Over => {
                            ((s + ((256 - u32::from(mask)) * d / 256)).min(255), d)
                        }
                        AssociatedBlend::Under if dst[3] == 0 => (s, s),
                        AssociatedBlend::Under => {
                            ((d + ((256 - u32::from(dst[3])) * s / 256)).min(255), s)
                        }
                    };
                    dst[channel] = ((composed * weight + base * (256 - weight)) / 256) as u8;
                }
                if options.coverage_write == CoverageWrite::Source {
                    dst[3] = mask;
                }
            }
        }
        Ok(())
    }

    /// Byte-exact RGB-only filtering with signed /256 floor; alpha is retained.
    pub fn filter_rgb(&mut self, rect: BitmapRect, filter: RgbFilter) -> Result<()> {
        self.validate()?;
        match filter {
            RgbFilter::Fade { weight, .. } => {
                ensure!(weight <= 256, "RGB fade weight must be 0..256")
            }
            RgbFilter::Multiply { factors } => ensure!(
                factors.iter().all(|&f| f <= 256),
                "RGB factors must be 0..256"
            ),
        }
        let x0 = i64::from(rect.x).max(0);
        let y0 = i64::from(rect.y).max(0);
        let x1 = (i64::from(rect.x) + i64::from(rect.width)).min(i64::from(self.width));
        let y1 = (i64::from(rect.y) + i64::from(rect.height)).min(i64::from(self.height));
        for y in y0..y1 {
            for x in x0..x1 {
                let at = ((y * i64::from(self.width) + x) * 4) as usize;
                for c in 0..3 {
                    let current = i32::from(self.rgba[at + c]);
                    self.rgba[at + c] = match filter {
                        RgbFilter::Fade { rgb, weight } => {
                            (current
                                + ((i32::from(rgb[c]) - current) * i32::from(weight))
                                    .div_euclid(256)) as u8
                        }
                        RgbFilter::Multiply { factors } => {
                            (current * i32::from(factors[c]) / 256) as u8
                        }
                    };
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(blend: AssociatedBlend, weight: u16) -> AssociatedBlitOptions {
        AssociatedBlitOptions {
            source_rect: BitmapRect::new(0, 0, 1, 1),
            destination: [0, 0],
            blend,
            weight,
            coverage_write: CoverageWrite::Preserve,
        }
    }
    #[test]
    fn external_coverage_and_mask_writes_do_not_use_display_alpha() {
        let src = ImageSurface::filled(1, 1, [64, 32, 16, 255]).unwrap();
        let mut dst = ImageSurface::filled(1, 1, [200, 100, 50, 71]).unwrap();
        let plane = CoveragePlane {
            width: 1,
            height: 1,
            bytes: vec![128],
        };
        let mut opt = options(AssociatedBlend::Over, 128);
        opt.coverage_write = CoverageWrite::Source;
        dst.composite_associated(&src, Some(&plane), opt).unwrap();
        assert_eq!(dst.rgba, [182, 91, 45, 128]);
    }
    #[test]
    fn under_uses_destination_coverage_and_source_as_fade_base() {
        let src = ImageSurface::filled(1, 1, [100, 50, 20, 9]).unwrap();
        let mut dst = ImageSurface::filled(1, 1, [20, 40, 60, 128]).unwrap();
        dst.composite_associated(&src, None, options(AssociatedBlend::Under, 128))
            .unwrap();
        assert_eq!(dst.rgba, [85, 57, 45, 128]);
        dst.rgba[3] = 0;
        dst.composite_associated(&src, None, options(AssociatedBlend::Under, 0))
            .unwrap();
        assert_eq!(dst.rgba, [100, 50, 20, 0]);
    }
    #[test]
    fn zero_mask_skip_and_full_weight_keep_native_byte_boundary() {
        let mut src = ImageSurface::filled(1, 1, [255, 0, 64, 0]).unwrap();
        let mut dst = ImageSurface::filled(1, 1, [255, 255, 255, 17]).unwrap();
        let before = dst.clone();
        dst.composite_associated(&src, None, options(AssociatedBlend::Over, 256))
            .unwrap();
        assert_eq!(dst, before);
        src.rgba[3] = 255;
        dst.composite_associated(&src, None, options(AssociatedBlend::Over, 256))
            .unwrap();
        assert_eq!(dst.rgba, [255, 0, 64, 17]);
    }
    #[test]
    fn invalid_requests_are_atomic_and_clipped_mapping_is_preserved() {
        let src = ImageSurface {
            width: 2,
            height: 1,
            rgba: vec![10, 20, 30, 40, 50, 60, 70, 80],
        };
        let mut dst = ImageSurface::filled(2, 1, [1, 2, 3, 4]).unwrap();
        let before = dst.clone();
        assert!(dst
            .composite_associated(&src, None, options(AssociatedBlend::Copy, 257))
            .is_err());
        assert_eq!(dst, before);
        let plane = CoveragePlane {
            width: 1,
            height: 1,
            bytes: vec![1],
        };
        assert!(dst
            .composite_associated(&src, Some(&plane), options(AssociatedBlend::Copy, 256))
            .is_err());
        assert_eq!(dst, before);
        let opt = AssociatedBlitOptions {
            source_rect: BitmapRect::new(-1, 0, 3, 1),
            destination: [-2, 0],
            coverage_write: CoverageWrite::Source,
            ..options(AssociatedBlend::Copy, 256)
        };
        dst.composite_associated(&src, None, opt).unwrap();
        assert_eq!(dst.rgba, [50, 60, 70, 80, 1, 2, 3, 4]);
    }
    #[test]
    fn signed_fade_floors_negative_deltas_and_never_changes_alpha() {
        let mut image = ImageSurface::filled(1, 1, [255, 1, 128, 13]).unwrap();
        image
            .filter_rgb(
                BitmapRect::new(0, 0, 1, 1),
                RgbFilter::Fade {
                    rgb: [0, 0, 129],
                    weight: 1,
                },
            )
            .unwrap();
        assert_eq!(image.rgba, [254, 0, 128, 13]);
        image
            .filter_rgb(
                BitmapRect::new(0, 0, 1, 1),
                RgbFilter::Multiply {
                    factors: [256, 128, 0],
                },
            )
            .unwrap();
        assert_eq!(image.rgba, [254, 0, 0, 13]);
        let before = image.clone();
        assert!(image
            .filter_rgb(
                BitmapRect::new(0, 0, 1, 1),
                RgbFilter::Multiply {
                    factors: [257, 0, 0]
                }
            )
            .is_err());
        assert_eq!(image, before);
    }
}

impl CoveragePlane {
    /// Resample byte coverage with nearest sampling anchored at the source origin.
    pub fn resize_nearest(&self, size: [u32; 2]) -> Result<Self> {
        self.validate()?;
        let count = crate::rgba_len(size[0], size[1])? / 4;
        let mut bytes = Vec::with_capacity(count);
        for y in 0..size[1] {
            let sy = (u64::from(y) * u64::from(self.height) / u64::from(size[1])) as usize;
            for x in 0..size[0] {
                let sx = (u64::from(x) * u64::from(self.width) / u64::from(size[0])) as usize;
                bytes.push(self.bytes[sy * self.width as usize + sx]);
            }
        }
        Ok(Self {
            width: size[0],
            height: size[1],
            bytes,
        })
    }
}

/// Replace RGBA alpha using a byte lookup over independent coverage, preserving
/// stored RGB. This supports hard/soft threshold wipes without a private painter.
pub fn map_coverage_alpha(pixels: &mut [u8], coverage: &[u8], lookup: &[u8; 256]) -> Result<()> {
    ensure!(
        pixels.len().is_multiple_of(4) && pixels.len() / 4 == coverage.len(),
        "coverage/pixel length mismatch"
    );
    ensure!(
        pixels.len() <= crate::MAX_DECODED_BYTES,
        "coverage byte budget"
    );
    for (pixel, value) in pixels.chunks_exact_mut(4).zip(coverage) {
        pixel[3] = lookup[*value as usize];
    }
    Ok(())
}

/// Byte threshold or clamped offset lookup for alpha wipes.
pub fn coverage_threshold_lut(hard: bool, phase: u16) -> [u8; 256] {
    std::array::from_fn(|a| {
        if hard {
            if a as u16 >= (phase & 255) {
                255
            } else {
                0
            }
        } else {
            (a as i32 + i32::from(phase) - 255).clamp(0, 255) as u8
        }
    })
}

#[cfg(test)]
mod coverage_mapping_tests {
    use super::*;
    #[test]
    fn scaling_and_thresholds_preserve_rgb_and_exact_pixel_alignment() {
        let mask = CoveragePlane {
            width: 2,
            height: 1,
            bytes: vec![0, 255],
        };
        let scaled = mask.resize_nearest([4, 2]).unwrap();
        assert_eq!(scaled.bytes, [0, 0, 255, 255, 0, 0, 255, 255]);
        let mut pixels = [11, 22, 33, 44].repeat(8);
        map_coverage_alpha(
            &mut pixels,
            &scaled.bytes,
            &coverage_threshold_lut(true, 128),
        )
        .unwrap();
        assert_eq!(
            &pixels[..16],
            &[11, 22, 33, 0, 11, 22, 33, 0, 11, 22, 33, 255, 11, 22, 33, 255]
        );
        assert_eq!(coverage_threshold_lut(false, 255)[128], 128);
        assert!(map_coverage_alpha(&mut pixels, &[0], &[0; 256]).is_err());
        assert!(mask.resize_nearest([0, 2]).is_err());
    }
}
