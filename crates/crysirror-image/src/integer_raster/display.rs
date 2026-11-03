//! Stored-byte post processing and fixed-point backdrop sampling.
use crate::ImageSurface as DecodedImage;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DisplayRaster {
    CoverageMask {
        origin: [i32; 2],
        channel: u8,
        replace: bool,
        attenuation: u16,
        retain_full: bool,
        bounds: Option<[u32; 2]>,
    },
    PerspectiveRaster {
        transform: super::perspective_blit::PerspectiveBlit,
        origin: [i32; 2],
    },
    InverseAffine {
        destination: [i32; 2],
        transform: [i32; 5],
        bilinear: bool,
    },
    DestinationFilter {
        enabled: bool,
        mode: i32,
        color: i32,
        level: u16,
        shift: i32,
        masked: bool,
        origin: [i32; 2],
    },
    TileCache {
        columns: u32,
        rows: u32,
        cell: [u32; 2],
        updates: Vec<(u32, u16)>,
    },
    Crossfade {
        amount: i32,
    },
    ThresholdMask {
        shift: u32,
        threshold: u32,
    },
    ColorFilters {
        filters: Vec<(u8, i32, u16)>,
    },
    HorizontalBlur {
        radius: u8,
        clamp: bool,
    },
    RotateBackdrop {
        scale: u32,
        angle: i32,
    },
    StretchBackdrop {
        rect: [f64; 4],
    },
    Mosaic {
        amount: i32,
        secondary: Option<u32>,
        coupled: bool,
    },
}
impl DisplayRaster {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::CoverageMask {
                origin,
                channel,
                attenuation,
                ..
            } => ensure!(
                *channel < 4
                    && *attenuation <= 256
                    && origin.iter().all(|v| v.unsigned_abs() <= 1_000_000),
                "stored coverage mask"
            ),
            Self::InverseAffine {
                destination,
                transform,
                ..
            } => ensure!(
                destination
                    .iter()
                    .chain(transform)
                    .all(|v| v.unsigned_abs() <= 0x7000_0000),
                "integer affine coordinates"
            ),
            Self::DestinationFilter {
                enabled,
                mode,
                level,
                masked,
                origin,
                ..
            } => ensure!(
                (!*enabled || *masked || matches!(*mode, 0..=3))
                    && *level <= 256
                    && origin.iter().all(|v| v.unsigned_abs() <= 1_000_000),
                "integer destination-reading filter"
            ),
            Self::TileCache {
                columns,
                rows,
                cell,
                updates,
            } => {
                let count = columns
                    .checked_mul(*rows)
                    .filter(|n| *n > 0)
                    .ok_or_else(|| anyhow::anyhow!("integer tile grid"))?;
                ensure!(
                    cell.iter().all(|n| (1..=256).contains(n))
                        && updates.len() as u64 <= u64::from(count)
                        && updates.iter().all(|(index, _)| *index < count),
                    "integer tile cache cells/updates"
                );
            }
            Self::Crossfade { amount } => {
                ensure!((0..=256).contains(amount), "integer crossfade amount")
            }
            Self::ThresholdMask { shift, .. } => {
                ensure!(*shift <= 31, "integer threshold mask shift")
            }
            Self::ColorFilters { filters } => ensure!(
                filters.len() <= 16 && filters.iter().all(|(kind, _, _)| *kind <= 5),
                "integer color filter stack"
            ),
            Self::StretchBackdrop { rect } => ensure!(
                rect.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.),
                "integer backdrop source rectangle"
            ),
            Self::Mosaic {
                amount, secondary, ..
            } => {
                ensure!((0..=256).contains(amount), "integer mosaic amount");
                ensure!(
                    secondary.is_none_or(|v| v == 0x7000 || v == 0x7001),
                    "integer mosaic solid background"
                );
            }
            _ => (),
        }
        Ok(())
    }
    pub fn apply(
        &self,
        primary: &DecodedImage,
        secondary: &DecodedImage,
        format: u32,
        size: [u32; 2],
        check: &dyn Fn() -> Result<()>,
    ) -> Result<DecodedImage> {
        self.validate()?;
        primary.validate()?;
        secondary.validate()?;
        crate::rgba_len(size[0], size[1])?;
        let mut output = match self {
            Self::CoverageMask {
                origin,
                channel,
                replace,
                attenuation,
                retain_full,
                bounds,
            } => {
                let mut image = primary.clone();
                let bounds = bounds.unwrap_or([secondary.width, secondary.height]);
                for y in 0..image.height {
                    check()?;
                    for x in 0..image.width {
                        let sx = i64::from(x) + i64::from(origin[0]);
                        let sy = i64::from(y) + i64::from(origin[1]);
                        let m = if sx >= 0
                            && sy >= 0
                            && sx < i64::from(secondary.width.min(bounds[0]))
                            && sy < i64::from(secondary.height.min(bounds[1]))
                        {
                            secondary.rgba[((sy as u32 * secondary.width + sx as u32) * 4) as usize
                                + usize::from(*channel)]
                        } else {
                            0
                        };
                        let at = ((y * image.width + x) * 4 + 3) as usize;
                        let weight = u32::from(m) * u32::from(256 - *attenuation);
                        image.rgba[at] = if *replace {
                            (weight >> 8) as u8
                        } else if *retain_full && weight == 65280 {
                            image.rgba[at]
                        } else {
                            ((weight * u32::from(image.rgba[at])) >> 16) as u8
                        };
                    }
                }
                image
            }

            Self::PerspectiveRaster { transform, origin } => {
                let mut image = primary.clone();
                let mut corners = super::perspective_blit::project_corners(
                    secondary.width,
                    secondary.height,
                    transform,
                );
                for corner in &mut corners {
                    corner[0] -= origin[0] as f32;
                    corner[1] -= origin[1] as f32;
                }
                let spans = super::perspective_blit::rasterize(&corners, size[1] as i32);
                ensure!(
                    super::perspective_blit::draw(
                        &mut image,
                        2,
                        &super::perspective_blit::Surface {
                            image: secondary,
                            format: 2
                        },
                        spans.as_ref(),
                        transform
                    ),
                    "projected raster formats"
                );
                image
            }

            Self::InverseAffine {
                destination,
                transform,
                bilinear,
            } => inverse_affine(secondary, size, *destination, *transform, *bilinear, check)?,
            Self::DestinationFilter {
                enabled,
                mode,
                color,
                level,
                shift,
                masked,
                origin,
            } => {
                let mut image = DecodedImage {
                    width: size[0],
                    height: size[1],
                    rgba: vec![0; crate::rgba_len(size[0], size[1])?],
                };
                for y in 0..image.height {
                    check()?;
                    for x in 0..image.width {
                        let sx = i64::from(x) + i64::from(origin[0]);
                        let sy = i64::from(y) + i64::from(origin[1]);
                        if sx < 0
                            || sy < 0
                            || sx >= i64::from(primary.width)
                            || sy >= i64::from(primary.height)
                        {
                            continue;
                        }
                        let src = ((sy as u32 * primary.width + sx as u32) * 4) as usize;
                        let dst = ((y * image.width + x) * 4) as usize;
                        image.rgba[dst..dst + 4].copy_from_slice(&primary.rgba[src..src + 4]);
                        let mask = if *masked {
                            Some(if x < secondary.width && y < secondary.height {
                                secondary.rgba[((y * secondary.width + x) * 4) as usize]
                            } else {
                                0
                            })
                        } else {
                            None
                        };
                        if *enabled {
                            filter_pixel(
                                &mut image.rgba[dst..dst + 4],
                                *mode,
                                *color,
                                i32::from(*level),
                                *shift,
                                mask,
                            );
                        }
                    }
                }
                image
            }
            Self::TileCache {
                columns,
                rows,
                cell: [cw, ch],
                updates,
            } => {
                ensure!(
                    columns.checked_mul(*cw) == Some(primary.width)
                        && rows.checked_mul(*ch) == Some(primary.height),
                    "integer tile cache size"
                );
                let across = secondary.width / cw;
                let count = across * (secondary.height / ch);
                ensure!(across > 0 && count > 0, "integer tile atlas");
                let mut image = primary.clone();
                for &(index, tile) in updates {
                    let x = index % columns;
                    let y = index / columns;
                    let tile = u32::from(tile);
                    for dy in 0..*ch {
                        check()?;
                        let dst = (((y * ch + dy) * image.width + x * cw) * 4) as usize;
                        let bytes = *cw as usize * 4;
                        if tile == 65535 || tile >= count {
                            image.rgba[dst..dst + bytes].fill(0);
                        } else {
                            let source = ((((tile / across) * ch + dy) * secondary.width
                                + (tile % across) * cw)
                                * 4) as usize;
                            image.rgba[dst..dst + bytes]
                                .copy_from_slice(&secondary.rgba[source..source + bytes]);
                        }
                    }
                }
                image
            }
            Self::Crossfade { amount } => {
                ensure!(
                    matches!(format, 1 | 2)
                        && primary.width == secondary.width
                        && primary.height == secondary.height,
                    "integer crossfade formats/size"
                );
                let mut image = primary.clone();
                for y in 0..primary.height {
                    check()?;
                    for x in 0..primary.width {
                        let at = ((y * primary.width + x) * 4) as usize;
                        let a = primary.rgba[at..at + 4].try_into().unwrap();
                        let b = secondary.rgba[at..at + 4].try_into().unwrap();
                        image.rgba[at..at + 4].copy_from_slice(
                            &super::bitmap_blend::crossfade_pixel(a, b, format as i32, *amount),
                        );
                    }
                }
                image
            }
            Self::ThresholdMask { shift, threshold } => {
                let mut image = primary.clone();
                let scale = 1i32.wrapping_shl(*shift).wrapping_add(1);
                let limit = (*threshold as i32).wrapping_mul(scale);
                for y in 0..primary.height {
                    check()?;
                    for x in 0..primary.width {
                        let mask = if x < secondary.width && y < secondary.height {
                            secondary.rgba[((y * secondary.width + x) * 4) as usize]
                        } else {
                            0
                        };
                        let coverage = if *threshold >= 256 {
                            256
                        } else {
                            limit
                                .wrapping_sub(i32::from(mask).wrapping_shl(*shift))
                                .clamp(0, 256)
                        };
                        let at = ((y * primary.width + x) * 4 + 3) as usize;
                        image.rgba[at] = if format == 1 {
                            coverage.min(255) as u8
                        } else {
                            (u32::from(image.rgba[at]) * coverage as u32 >> 8) as u8
                        };
                    }
                }
                image
            }
            Self::ColorFilters { filters } => {
                let mut image = primary.clone();
                for &(kind, color, strength) in filters {
                    apply_color_filter(&mut image, kind, color, strength, check)?;
                }
                image
            }
            Self::HorizontalBlur { radius, clamp } => {
                native_horizontal_blur(primary, i32::from(*radius), *clamp, check)?
            }
            Self::RotateBackdrop { scale, angle } => {
                native_rotate_background(primary, size, *scale, *angle, check)?
            }
            Self::StretchBackdrop { rect } => {
                native_stretch_background(primary, size, *rect, check)?
            }
            Self::Mosaic {
                amount,
                secondary: solid,
                coupled,
            } => {
                let mut image = DecodedImage {
                    width: size[0],
                    height: size[1],
                    rgba: vec![0; crate::rgba_len(size[0], size[1])?],
                };
                if *coupled {
                    if let Some(color) = solid {
                        let pixel = if *color == 0x7001 {
                            [255; 4]
                        } else {
                            [0, 0, 0, 255]
                        };
                        for p in image.rgba.chunks_exact_mut(4) {
                            p.copy_from_slice(&pixel);
                        }
                    } else {
                        native_mosaic_into(&mut image, secondary, 256 - *amount, 0, check)?;
                    }
                }
                native_mosaic_into(
                    &mut image,
                    primary,
                    *amount,
                    if *coupled { *amount } else { 0 },
                    check,
                )?;
                image
            }
        };
        if matches!(
            self,
            Self::HorizontalBlur { .. }
                | Self::RotateBackdrop { .. }
                | Self::StretchBackdrop { .. }
                | Self::Mosaic { .. }
        ) {
            for p in output.rgba.chunks_exact_mut(4) {
                p[3] = 255;
            }
        }
        check()?;
        Ok(output)
    }
}
fn native_horizontal_blur(
    source: &DecodedImage,
    alpha: i32,
    clamp: bool,
    check: &dyn Fn() -> Result<()>,
) -> Result<DecodedImage> {
    let radius = (alpha as u32).min(255) as i32;
    if radius == 0 {
        return Ok(source.clone());
    }
    let count = 2 * radius + 1;
    let shift = if count < 64 {
        0
    } else if count < 128 {
        1
    } else {
        2
    };
    let reciprocal = 0x10003 / count;
    let mut output = source.clone();
    for y in 0..source.height as usize {
        check()?;
        let sample = |x: i32, c: usize| -> i32 {
            let x = if clamp {
                x.clamp(0, source.width as i32 - 1)
            } else if x < 0 || x >= source.width as i32 {
                return 0;
            } else {
                x
            };
            i32::from(source.rgba[(y * source.width as usize + x as usize) * 4 + c]) >> shift
        };
        let mut sums = [0i32; 4];
        for dx in -radius..=radius {
            for c in 0..4 {
                sums[c] += sample(dx, c);
            }
        }
        for x in 0..source.width as i32 {
            for c in 0..4 {
                output.rgba[(y * source.width as usize + x as usize) * 4 + c] =
                    ((((sums[c] as i16 as i32) * reciprocal) >> 16) << shift).clamp(0, 255) as u8;
                sums[c] -= sample(x - radius, c);
                sums[c] += sample(x + radius + 1, c);
            }
        }
    }
    Ok(output)
}

fn native_mosaic_into(
    destination: &mut DecodedImage,
    source: &DecodedImage,
    block_parameter: i32,
    transparency: i32,
    check: &dyn Fn() -> Result<()>,
) -> Result<()> {
    let block = (block_parameter as u32).saturating_add(1).max(1);
    for y in 0..destination.height {
        check()?;
        for x in 0..destination.width {
            let sx = (x / block * block).min(source.width - 1);
            let sy = (y / block * block).min(source.height - 1);
            let src = (sy as usize * source.width as usize + sx as usize) * 4;
            let dst = (y as usize * destination.width as usize + x as usize) * 4;
            for c in 0..4 {
                let s = i32::from(source.rgba[src + c]);
                let d = i32::from(destination.rgba[dst + c]);
                destination.rgba[dst + c] = (s + ((d - s) * transparency >> 8)).clamp(0, 255) as u8;
            }
        }
    }
    Ok(())
}

fn native_rotate_background(
    source: &DecodedImage,
    size: [u32; 2],
    scale: u32,
    angle: i32,
    check: &dyn Fn() -> Result<()>,
) -> Result<DecodedImage> {
    let mut image = DecodedImage {
        width: size[0],
        height: size[1],
        rgba: vec![0; size[0] as usize * size[1] as usize * 4],
    };
    if scale == 0 {
        return Ok(image);
    }
    let theta = -(angle as f64) * std::f64::consts::PI / (180.0 * 65536.0);
    let (sin, cos) = theta.sin_cos();
    let inverse = 65536.0 / scale as f64;
    let left = -((size[0] - 1) as f64) * 0.5;
    let top = (size[1] - 1) as f64 * 0.5;
    let start_x = ((left * cos - top * sin) * inverse * 65536.0 + source.width as f64 * 32768.0)
        as i64 as i32;
    let start_y = ((top * cos + left * sin) * -inverse * 65536.0 + source.height as f64 * 32768.0)
        as i64 as i32;
    let dx = [
        (cos * inverse * 65536.0) as i64 as i32,
        (-sin * inverse * 65536.0) as i64 as i32,
    ];
    let dy = [
        (sin * inverse * 65536.0) as i64 as i32,
        (cos * inverse * 65536.0) as i64 as i32,
    ];
    for y in 0..size[1] {
        check()?;
        for x in 0..size[0] {
            let sx = start_x
                .wrapping_add(dx[0].wrapping_mul(x as i32))
                .wrapping_add(dy[0].wrapping_mul(y as i32));
            let sy = start_y
                .wrapping_add(dx[1].wrapping_mul(x as i32))
                .wrapping_add(dy[1].wrapping_mul(y as i32));
            if sx >= 0
                && sy >= 0
                && (sx >> 16) < source.width as i32
                && (sy >> 16) < source.height as i32
            {
                let src = ((sy as usize >> 16) * source.width as usize + (sx as usize >> 16)) * 4;
                let dst = (y as usize * size[0] as usize + x as usize) * 4;
                image.rgba[dst..dst + 4].copy_from_slice(&source.rgba[src..src + 4]);
            }
        }
    }
    Ok(image)
}

fn native_stretch_background(
    source: &DecodedImage,
    size: [u32; 2],
    rect: [f64; 4],
    check: &dyn Fn() -> Result<()>,
) -> Result<DecodedImage> {
    let mut image = DecodedImage {
        width: size[0],
        height: size[1],
        rgba: vec![0; size[0] as usize * size[1] as usize * 4],
    };
    let [x, y, w, h] = rect;
    if w < 2.0 || h < 2.0 {
        return Ok(image);
    }
    let scale_x = (source.width.wrapping_mul(0x10004) as f64 / w) as u32;
    let scale_y = (source.height.wrapping_mul(0x10004) as f64 / h) as u32;
    if scale_x == 0 || scale_y == 0 {
        return Ok(image);
    }
    let ix = 65536.0 / scale_x as f64;
    let iy = 65536.0 / scale_y as f64;
    let start = [
        (x * 65536.0 + w * 32768.0 - size[0] as f64 * 32768.0 * ix
            + if scale_x < 65536 {
                (ix * 0.5 - 0.5) * 65536.0
            } else {
                0.0
            }) as i64 as i32,
        (y * 65536.0 + h * 32768.0 - size[1] as f64 * 32768.0 * iy
            + if scale_y < 65536 {
                (iy * 0.5 - 0.5) * 65536.0
            } else {
                0.0
            }) as i64 as i32,
    ];
    let step = [(ix * 65536.0) as i32, (iy * 65536.0) as i32];
    for y in 0..size[1] {
        check()?;
        for x in 0..size[0] {
            let sx = start[0].wrapping_add(step[0].wrapping_mul(x as i32));
            let sy = start[1].wrapping_add(step[1].wrapping_mul(y as i32));
            let p = sample_background_pixel(
                source,
                sx >> 16,
                sy >> 16,
                ((sx as u32 >> 9) & 127) as i32,
                ((sy as u32 >> 9) & 127) as i32,
                128,
            );
            let dst = (y as usize * size[0] as usize + x as usize) * 4;
            image.rgba[dst..dst + 4].copy_from_slice(&p);
        }
    }
    Ok(image)
}

pub(crate) fn sample_background_pixel(
    source: &DecodedImage,
    x: i32,
    y: i32,
    fx: i32,
    fy: i32,
    denominator: i32,
) -> [u8; 4] {
    let pixel = |x: i32, y: i32| -> [u8; 4] {
        if x < 0 || y < 0 || x >= source.width as i32 || y >= source.height as i32 {
            return [0; 4];
        }
        let i = (y as usize * source.width as usize + x as usize) * 4;
        source.rgba[i..i + 4].try_into().unwrap()
    };
    let a = pixel(x, y);
    let b = pixel(x + 1, y);
    let c = pixel(x, y + 1);
    let d = pixel(x + 1, y + 1);
    let shift = denominator.trailing_zeros();
    std::array::from_fn(|i| {
        let top =
            i32::from(b[i]) + ((i32::from(a[i]) - i32::from(b[i])) * (denominator - fx) >> shift);
        let bottom =
            i32::from(d[i]) + ((i32::from(c[i]) - i32::from(d[i])) * (denominator - fx) >> shift);
        (bottom + ((top - bottom) * (denominator - fy) >> shift)).clamp(0, 255) as u8
    })
}

fn apply_color_filter(
    image: &mut DecodedImage,
    kind: u8,
    color: i32,
    strength: u16,
    check: &dyn Fn() -> Result<()>,
) -> Result<()> {
    let color = [
        (color as u32 >> 16) as u8,
        (color as u32 >> 8) as u8,
        color as u8,
    ];
    for (index, pixel) in image.rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        if index % 16384 == 0 {
            check()?;
        }
        let luma = 77 * u32::from(pixel[0]) + 150 * u32::from(pixel[1]) + 29 * u32::from(pixel[2]);
        for lane in 0..3 {
            let d = i16::from(pixel[lane]);
            let c = u16::from(color[lane]);
            let value = match kind {
                1 => {
                    let target = d ^ c as i16;
                    let weight = (strength as i16) >> 1;
                    d.wrapping_add(target.wrapping_sub(d).wrapping_mul(weight) >> 7) as i32
                }
                2 => {
                    let target = ((luma * u32::from(c)) >> 16) as i16;
                    let weight = (strength >> 1) as i16;
                    d.wrapping_add(target.wrapping_sub(d).wrapping_mul(weight) >> 7) as i32
                }
                3 => {
                    let base = (d as u16).wrapping_mul(256u16.wrapping_sub(strength));
                    i32::from(base.saturating_add(c.wrapping_mul(strength)) >> 8)
                }
                4 => i32::from(d) + i32::from(c.wrapping_mul(strength) >> 8),
                5 => i32::from(d) - i32::from(c.wrapping_mul(strength) >> 8),
                _ => i32::from(d),
            };
            pixel[lane] = value.clamp(0, 255) as u8;
        }
    }
    Ok(())
}

fn filter_pixel(pixel: &mut [u8], mode: i32, color: i32, p: i32, shift: i32, mask: Option<u8>) {
    let color = [
        ((color as u32 >> 16) & 255) as i32,
        ((color as u32 >> 8) & 255) as i32,
        (color as u32 & 255) as i32,
    ];
    if let Some(mask) = mask {
        let shift = (shift as u32) & 31;
        let keep = 256i32
            .wrapping_sub(p.wrapping_mul(1i32.wrapping_shl(shift).wrapping_add(1)))
            .wrapping_add(i32::from(mask).wrapping_shl(shift));
        if keep >= 256 {
            return;
        }
        for c in 0..3 {
            pixel[c] = if keep <= 0 {
                color[c] as u8
            } else {
                (color[c] + ((i32::from(pixel[c]) - color[c]) * (keep >> 1) >> 7)).clamp(0, 255)
                    as u8
            };
        }
        return;
    }
    let luminance = 77 * i32::from(pixel[0]) + 150 * i32::from(pixel[1]) + 29 * i32::from(pixel[2]);
    for c in 0..3 {
        let old = i32::from(pixel[c]);
        let value = match mode {
            0 => (old * (256 - p) + color[c] * p) >> 8,
            1 => {
                if color == [0, 0, 0] {
                    old - ((255 * p) >> 8)
                } else {
                    old + ((color[c] * p) >> 8)
                }
            }
            2 => old + (((luminance * color[c] >> 16) - old) * (p >> 1) >> 7),
            3 => old + (((old ^ color[c]) - old) * (p >> 1) >> 7),
            _ => old,
        };
        pixel[c] = value.clamp(0, 255) as u8;
    }
}

fn inverse_affine(
    source: &DecodedImage,
    size: [u32; 2],
    destination: [i32; 2],
    t: [i32; 5],
    bilinear: bool,
    check: &dyn Fn() -> Result<()>,
) -> Result<DecodedImage> {
    let [anchor_x, anchor_y, angle, scale_x, scale_y] = t;
    let mut out = DecodedImage {
        width: size[0],
        height: size[1],
        rgba: vec![0; crate::rgba_len(size[0], size[1])?],
    };
    if scale_x == 0 || scale_y == 0 {
        return Ok(out);
    }
    let theta = -(angle as f64) * std::f64::consts::PI / 11796480.;
    let (sin, cos) = theta.sin_cos();
    let (sx, sy) = (
        65536. / (scale_x as u32 as f64),
        65536. / (scale_y as u32 as f64),
    );
    let cx = destination[0] as f64 / 65536.;
    let cy = destination[1] as f64 / 65536.;
    let start = [
        (((-cx * cos - cy * sin) * sx + anchor_x as f64 / 65536.) * 65536.) as i64 as i32,
        ((anchor_y as f64 / 65536. - (cy * cos - cx * sin) * sy) * 65536.) as i64 as i32,
    ];
    let dx = [
        (cos * sx * 65536.) as i64 as i32,
        (-sin * sy * 65536.) as i64 as i32,
    ];
    let reverse = std::f64::consts::FRAC_PI_2 - angle as f64 * std::f64::consts::PI / 11796480.;
    let dy = [
        (-reverse.cos() * sx * 65536.) as i64 as i32,
        (reverse.sin() * sy * 65536.) as i64 as i32,
    ];
    for y in 0..out.height {
        check()?;
        for x in 0..out.width {
            let fx = start[0]
                .wrapping_add(dx[0].wrapping_mul(x as i32))
                .wrapping_add(dy[0].wrapping_mul(y as i32));
            let fy = start[1]
                .wrapping_add(dx[1].wrapping_mul(x as i32))
                .wrapping_add(dy[1].wrapping_mul(y as i32));
            let (px, py) = (
                (fx >> 16).clamp(-32768, 32767),
                (fy >> 16).clamp(-32768, 32767),
            );
            let color = sample_background_pixel(
                source,
                px,
                py,
                if bilinear {
                    (fx as u32 >> 12 & 15) as i32
                } else {
                    0
                },
                if bilinear {
                    (fy as u32 >> 12 & 15) as i32
                } else {
                    0
                },
                16,
            );
            let index = ((y * out.width + x) * 4) as usize;
            out.rgba[index..index + 4].copy_from_slice(&color);
        }
    }
    Ok(out)
}

/// Conservative inclusive sample footprint for a fixed-point affine image.
/// The returned rectangle is relative to the caller's object origin. Bilinear
/// coverage includes the one-pixel zero border consumed by stored-byte kernels.
pub fn affine_sample_bounds(
    size: [u32; 2],
    pivot: [i32; 2],
    angle: i32,
    factors: [u32; 2],
    filtered: bool,
) -> Result<[i32; 4]> {
    crate::rgba_len(size[0], size[1])?;
    ensure!(
        factors.iter().all(|v| *v != 0 && *v <= 0x1000_0000),
        "affine footprint scale"
    );
    let (sin, cos) = (f64::from(angle) * std::f64::consts::PI / 11_796_480.).sin_cos();
    let [sx, sy] = factors.map(|v| f64::from(v) / 65536.);
    let border = if filtered { 1. } else { 0. };
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for [x, y] in [
        [-border, -border],
        [-border, f64::from(size[1]) - 1. + border],
        [
            f64::from(size[0]) - 1. + border,
            f64::from(size[1]) - 1. + border,
        ],
        [f64::from(size[0]) - 1. + border, -border],
    ] {
        let x = (x - f64::from(pivot[0])) * sx;
        let y = (y - f64::from(pivot[1])) * sy;
        let ox = f64::from(pivot[0]) + x * cos - y * sin;
        let oy = f64::from(pivot[1]) + x * sin + y * cos;
        bounds[0] = bounds[0].min(ox);
        bounds[1] = bounds[1].min(oy);
        bounds[2] = bounds[2].max(ox);
        bounds[3] = bounds[3].max(oy);
    }
    ensure!(
        bounds
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 1_000_000.),
        "affine footprint coordinates"
    );
    let [left, top, right, bottom] = [
        bounds[0].floor() as i32,
        bounds[1].floor() as i32,
        bounds[2].ceil() as i32,
        bounds[3].ceil() as i32,
    ];
    Ok([left, top, right - left + 1, bottom - top + 1])
}
