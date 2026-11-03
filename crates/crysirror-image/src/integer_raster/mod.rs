//! Integer bitmap arithmetic, fixed-point sampling and projected quad rasterization.
//! These operations work on individual engine images, independently of a VM.
pub mod bitmap_blend;
pub mod bitmap_kernels;
pub mod display;
pub mod graph_color;
pub mod perspective_blit;

use crate::ImageSurface;
use anyhow::{Result, ensure};
use bitmap_kernels::TargetBitmap;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum IntegerRasterOperation {
    Mirror {
        vertical: bool,
    },
    HalfBox,
    AspectFit,
    Display {
        spec: display::DisplayRaster,
    },
    CrossfadeAt {
        position: [i32; 2],
        amount: u16,
    },
    StretchCopy {
        source: [i32; 4],
        destination: [i32; 4],
    },
    ReplaceRgb {
        color: u32,
    },
    CopyRect {
        source: [i32; 4],
        position: [i32; 2],
    },
    Blit {
        position: [i32; 2],
        mode: i32,
        parameter: i32,
    },
    Scale {
        factors: [u32; 2],
        filtered: bool,
    },
    Concentrate {
        attenuation: u32,
        factors: [u32; 2],
    },
    Process {
        kind: u32,
        color: u32,
        alpha: u32,
    },
    GrayMask {
        offset: [i32; 2],
    },
    Rotate {
        positions: [i32; 4],
        angle: i32,
        factors: [u32; 2],
        alpha: u32,
        filtered: bool,
        copy: bool,
    },
    ToneCurve {
        points: [[i32; 2]; 3],
        mono_color: u32,
        mono_level: u32,
        film_color: u32,
        film_level: u32,
    },
    Perspective {
        transform: perspective_blit::PerspectiveBlit,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegerRaster {
    /// 0: RGB555 expansion, 1: stored XRGB, 2: straight ARGB, 3: red-channel coverage.
    pub destination_format: u32,
    pub source_format: u32,
    pub raster: IntegerRasterOperation,
}
impl IntegerRaster {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (0..=3).contains(&self.destination_format) && (0..=3).contains(&self.source_format),
            "integer raster pixel format"
        );
        match &self.raster {
            IntegerRasterOperation::Display { spec } => spec.validate()?,
            IntegerRasterOperation::Process { kind, alpha, .. } => ensure!(
                *kind <= 5 && *alpha <= 256,
                "integer bitmap process kind/level"
            ),
            IntegerRasterOperation::CopyRect { source, position } => ensure!(
                source
                    .iter()
                    .chain(position)
                    .all(|v| v.unsigned_abs() <= 1_000_000),
                "integer copy rectangle"
            ),
            IntegerRasterOperation::Blit {
                position,
                mode,
                parameter,
            } => {
                ensure!(
                    position.iter().all(|v| v.unsigned_abs() <= 1_000_000),
                    "integer blit position"
                );
                ensure!(
                    matches!(*mode, 0..=9 | 0x20..=0x27 | 0x40 | 0x41 | 0x80 | 0xc0 | 0xc1 | 0xf0 | 0xff),
                    "integer blit mode"
                );
                ensure!((0..=256).contains(parameter), "integer blit parameter");
            }
            IntegerRasterOperation::Scale { factors, .. }
            | IntegerRasterOperation::Concentrate { factors, .. }
            | IntegerRasterOperation::Rotate { factors, .. } => {
                ensure!(
                    factors.iter().all(|&v| v <= 0x1000_0000),
                    "integer scale factors"
                );
            }
            IntegerRasterOperation::ToneCurve { points, .. } => {
                ensure!(
                    graph_color::RuntimeColorLut::from_control_points(*points).is_some(),
                    "integer tone curve points"
                );
            }
            _ => (),
        }
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
        ensure!(
            u64::from(destination.width) * u64::from(destination.height)
                <= crate::effects::MAX_EFFECT_PIXELS
                && u64::from(source.width) * u64::from(source.height)
                    <= crate::effects::MAX_EFFECT_PIXELS,
            "integer raster pixel budget"
        );
        check()?;
        let mut output = destination.clone();
        let df = self.destination_format;
        let sf = self.source_format;
        match &self.raster {
            IntegerRasterOperation::Display { spec } => {
                let primary = if matches!(
                    spec,
                    display::DisplayRaster::RotateBackdrop { .. }
                        | display::DisplayRaster::StretchBackdrop { .. }
                        | display::DisplayRaster::HorizontalBlur { .. }
                ) {
                    source
                } else {
                    destination
                };
                output = spec.apply(
                    primary,
                    source,
                    df,
                    [destination.width, destination.height],
                    check,
                )?;
            }
            IntegerRasterOperation::Mirror { vertical } => {
                ensure!(
                    output.width == source.width && output.height == source.height,
                    "mirror output size"
                );
                for y in 0..source.height {
                    check()?;
                    for x in 0..source.width {
                        let sx = if *vertical { x } else { source.width - 1 - x };
                        let sy = if *vertical { source.height - 1 - y } else { y };
                        let si = ((sy * source.width + sx) * 4) as usize;
                        let di = ((y * output.width + x) * 4) as usize;
                        output.rgba[di..di + 4].copy_from_slice(&source.rgba[si..si + 4]);
                    }
                }
            }
            IntegerRasterOperation::HalfBox => {
                ensure!(
                    output.width == source.width.div_ceil(2)
                        && output.height == source.height.div_ceil(2),
                    "half-box output size"
                );
                for y in 0..output.height {
                    check()?;
                    for x in 0..output.width {
                        let mut sum = [0u32; 4];
                        let mut count = 0;
                        for sy in y * 2..(y * 2 + 2).min(source.height) {
                            for sx in x * 2..(x * 2 + 2).min(source.width) {
                                let at = ((sy * source.width + sx) * 4) as usize;
                                for c in 0..4 {
                                    sum[c] += u32::from(source.rgba[at + c]);
                                }
                                count += 1;
                            }
                        }
                        let di = ((y * output.width + x) * 4) as usize;
                        for c in 0..4 {
                            output.rgba[di + c] = (sum[c] / count) as u8;
                        }
                    }
                }
            }
            IntegerRasterOperation::AspectFit => {
                let scale = ((u64::from(output.width) << 16) / u64::from(source.width))
                    .min((u64::from(output.height) << 16) / u64::from(source.height))
                    .max(1);
                let w = ((u64::from(source.width) * scale) >> 16) as u32;
                let h = ((u64::from(source.height) * scale) >> 16) as u32;
                if w > 0 && h > 0 {
                    output.rgba.fill(0);
                    let dx = (output.width - w) / 2;
                    let dy = (output.height - h) / 2;
                    for y in 0..h {
                        check()?;
                        let sy =
                            ((u64::from(y) << 16) / scale).min(u64::from(source.height - 1)) as u32;
                        for x in 0..w {
                            let sx = ((u64::from(x) << 16) / scale).min(u64::from(source.width - 1))
                                as u32;
                            let si = ((sy * source.width + sx) * 4) as usize;
                            let di = (((dy + y) * output.width + dx + x) * 4) as usize;
                            output.rgba[di..di + 4].copy_from_slice(&source.rgba[si..si + 4]);
                        }
                    }
                }
            }
            IntegerRasterOperation::CrossfadeAt { position, amount } => {
                ensure!(*amount <= 256, "integer crossfade level");
                for y in 0..source.height {
                    check()?;
                    let dy = i64::from(y) + i64::from(position[1]);
                    if !(0..i64::from(output.height)).contains(&dy) {
                        continue;
                    }
                    for x in 0..source.width {
                        let dx = i64::from(x) + i64::from(position[0]);
                        if !(0..i64::from(output.width)).contains(&dx) {
                            continue;
                        }
                        let si = ((y * source.width + x) * 4) as usize;
                        let di = ((dy as u32 * output.width + dx as u32) * 4) as usize;
                        let pixel = bitmap_blend::crossfade_pixel(
                            output.rgba[di..di + 4].try_into().unwrap(),
                            source.rgba[si..si + 4].try_into().unwrap(),
                            df as i32,
                            i32::from(*amount),
                        );
                        output.rgba[di..di + 4].copy_from_slice(&pixel);
                    }
                }
            }
            IntegerRasterOperation::StretchCopy {
                source: region,
                destination: target,
            } => {
                region::stretch_copy(&mut output, source, *region, *target, check)?;
            }
            IntegerRasterOperation::ReplaceRgb { color } => {
                let [r, g, b] = [(color >> 16) as u8, (color >> 8) as u8, *color as u8];
                for y in 0..destination.height.min(source.height) {
                    check()?;
                    for x in 0..destination.width.min(source.width) {
                        let dst = ((y * destination.width + x) * 4) as usize;
                        let src = ((y * source.width + x) * 4) as usize;
                        output.rgba[dst..dst + 4].copy_from_slice(&[r, g, b, source.rgba[src + 3]]);
                    }
                }
            }
            IntegerRasterOperation::CopyRect {
                source: rect,
                position,
            } => {
                let left = rect[0].max(0);
                let top = rect[1].max(0);
                let right = (i64::from(rect[0]) + i64::from(rect[2])).min(i64::from(source.width));
                let bottom =
                    (i64::from(rect[1]) + i64::from(rect[3])).min(i64::from(source.height));
                if right > i64::from(left) && bottom > i64::from(top) {
                    let width = (right - i64::from(left)) as u32;
                    let height = (bottom - i64::from(top)) as u32;
                    let mut rgba = Vec::new();
                    rgba.try_reserve_exact(crate::rgba_len(width, height)?)?;
                    for row in 0..height {
                        check()?;
                        let at = (((top as u32 + row) * source.width + left as u32) * 4) as usize;
                        rgba.extend_from_slice(&source.rgba[at..at + width as usize * 4]);
                    }
                    let region = ImageSurface {
                        width,
                        height,
                        rgba,
                    };
                    ensure!(
                        bitmap_blend::blit_copy(
                            &mut output,
                            df as i32,
                            &region,
                            sf as i32,
                            position[0],
                            position[1]
                        ),
                        "integer copy rectangle formats"
                    );
                }
            }
            IntegerRasterOperation::Blit {
                position: [x, y],
                mode,
                parameter,
            } => {
                ensure!(
                    bitmap_blend::blit_mode(
                        &mut output,
                        df as i32,
                        source,
                        sf as i32,
                        *x,
                        *y,
                        *mode,
                        *parameter
                    ),
                    "integer bitmap blend has no kernel for these formats/mode"
                );
            }
            IntegerRasterOperation::Perspective { transform } => {
                let corners =
                    perspective_blit::project_corners(source.width, source.height, transform);
                let spans = perspective_blit::rasterize(&corners, destination.height as i32);
                ensure!(
                    perspective_blit::draw(
                        &mut output,
                        df as i32,
                        &perspective_blit::Surface {
                            image: source,
                            format: sf as i32
                        },
                        spans.as_ref(),
                        transform
                    ),
                    "integer projected bitmap formats/mode"
                );
            }
            raster => {
                let mut dst = if df == 3 {
                    TargetBitmap::gray_from_image(destination)
                } else {
                    TargetBitmap::from_image(destination)
                };
                let src = if sf == 3 {
                    TargetBitmap::gray_from_image(source)
                } else {
                    TargetBitmap::from_image(source)
                };
                match raster {
                    IntegerRasterOperation::Scale {
                        factors: [x, y],
                        filtered,
                    } => {
                        if *filtered {
                            bitmap_kernels::scale_filtered(&mut dst, &src, sf, *x, *y);
                        } else {
                            bitmap_kernels::scale_nearest(&mut dst, &src, *x, *y);
                        }
                    }
                    IntegerRasterOperation::Concentrate {
                        attenuation,
                        factors: [x, y],
                    } => bitmap_kernels::concentrate(&mut dst, &src, *attenuation, *x, *y),
                    IntegerRasterOperation::Process { kind, color, alpha } => {
                        bitmap_kernels::process(&mut dst, df, &src, sf, *kind, *color, *alpha)
                    }
                    IntegerRasterOperation::GrayMask { offset: [x, y] } => {
                        bitmap_kernels::grayscale_mask(&mut dst, df, &src, sf, *x, *y)
                    }
                    IntegerRasterOperation::Rotate {
                        positions,
                        angle,
                        factors: [x, y],
                        alpha,
                        filtered,
                        copy,
                    } => {
                        if *copy {
                            bitmap_kernels::rotate_copy(
                                &mut dst, df, &src, sf, *positions, *angle, *x, *y, *alpha,
                                *filtered,
                            );
                        } else {
                            bitmap_kernels::rotate_blit(
                                &mut dst, df, &src, sf, *positions, *angle, *x, *y, *alpha,
                                *filtered,
                            );
                        }
                    }
                    IntegerRasterOperation::ToneCurve {
                        points,
                        mono_color,
                        mono_level,
                        film_color,
                        film_level,
                    } => {
                        let lut = graph_color::RuntimeColorLut::from_control_points(*points)
                            .expect("validated curve");
                        bitmap_kernels::tone_curve(
                            &mut dst,
                            &src,
                            sf,
                            lut.lane_tables(),
                            *mono_color,
                            *mono_level,
                            *film_color,
                            *film_level,
                        );
                    }
                    _ => unreachable!("direct raster handled above"),
                }
                output = if df == 3 {
                    dst.gray_to_image()
                } else {
                    dst.to_image(df == 1)
                };
            }
        }
        check()?;
        Ok(output)
    }
}

pub mod region;
