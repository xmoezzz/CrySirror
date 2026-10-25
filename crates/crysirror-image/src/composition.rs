// SPDX-License-Identifier: MPL-2.0
//! Shared float-coordinate CPU composition. Byte arithmetic and sampling profiles
//! are explicit; compatibility layout, native handles and VM execution stay outside.
//! Recovered profiles follow ethornell-rs core::composite_native_rgba and snapshot.rs
//! at 4c41dc3a566338d2ea8400635e23b062613e3782; no whole-frame native renderer is called.
use crate::{rgba_len, ImageSurface};
use anyhow::{ensure, Result};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatBlend {
    Coverage,
    Add,
    Subtract,
    ConstantInterpolation256,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatSampling {
    NearestClamp,
    NearestTransparent,
    Bilinear16Transparent,
}
#[derive(Clone, Copy, Debug)]
pub struct ImageView<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
}
impl<'a> ImageView<'a> {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            rgba_len(self.width, self.height)? == self.rgba.len(),
            "invalid image view"
        );
        Ok(())
    }
    fn pixel(&self, x: i32, y: i32) -> Option<[u8; 4]> {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return None;
        }
        let at = ((y as u32 * self.width + x as u32) * 4) as usize;
        Some(self.rgba[at..at + 4].try_into().unwrap())
    }
}
impl<'a> From<&'a ImageSurface> for ImageView<'a> {
    fn from(s: &'a ImageSurface) -> Self {
        Self {
            width: s.width,
            height: s.height,
            rgba: &s.rgba,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct FloatDraw {
    pub destination: [f32; 4],
    pub source: [f32; 4],
    pub opacity: f32,
    pub ignore_source_alpha: bool,
    pub blend: FloatBlend,
    /// Optional affine destination vertices, TL, BL, BR, TR.
    pub quad: Option<[[f32; 2]; 4]>,
    pub sampling: FloatSampling,
    pub clip: Option<[f32; 4]>,
}
/// Gamma-encoded channel math: recovered coverage blend ignores destination alpha
/// in RGB interpolation, unlike conventional straight-alpha Porter-Duff source-over.
pub fn blend_pixel(destination: &mut [u8; 4], source: [u8; 4], opacity: f32, mode: FloatBlend) {
    let opacity = opacity.clamp(0., 1.);
    if mode == FloatBlend::ConstantInterpolation256 {
        let t = ((1. - opacity) * 256.).round().clamp(0., 256.) as i32;
        for c in 0..4 {
            destination[c] = (i32::from(source[c])
                + (((i32::from(destination[c]) - i32::from(source[c])) * t) >> 8))
                .clamp(0, 255) as u8;
        }
        return;
    }
    if opacity == 1. && source[3] == 255 && mode == FloatBlend::Coverage {
        *destination = source;
        return;
    }
    let alpha = (source[3] as f32 / 255.) * opacity;
    if alpha <= 0. {
        return;
    }
    for c in 0..3 {
        let value = match mode {
            FloatBlend::Coverage => source[c] as f32 * alpha + destination[c] as f32 * (1. - alpha),
            FloatBlend::Add => destination[c] as f32 + source[c] as f32 * alpha,
            FloatBlend::Subtract => destination[c] as f32 - source[c] as f32 * alpha,
            FloatBlend::ConstantInterpolation256 => unreachable!(),
        };
        destination[c] = value.round().clamp(0., 255.) as u8;
    }
    let da = destination[3] as f32 / 255.;
    destination[3] = ((alpha + da * (1. - alpha)) * 255.).round().clamp(0., 255.) as u8;
}
fn sample(src: ImageView<'_>, x: f32, y: f32, sampling: FloatSampling) -> Option<[u8; 4]> {
    match sampling {
        FloatSampling::NearestClamp => src.pixel(
            x.clamp(0., src.width.saturating_sub(1) as f32) as i32,
            y.clamp(0., src.height.saturating_sub(1) as f32) as i32,
        ),
        FloatSampling::NearestTransparent => src.pixel(x.floor() as i32, y.floor() as i32),
        FloatSampling::Bilinear16Transparent => {
            let x0 = x.floor() as i32;
            let y0 = y.floor() as i32;
            let fx = ((x - x.floor()) * 16.).floor().clamp(0., 15.) / 16.;
            let fy = ((y - y.floor()) * 16.).floor().clamp(0., 15.) / 16.;
            let p = [
                src.pixel(x0, y0).unwrap_or([0; 4]),
                src.pixel(x0 + 1, y0).unwrap_or([0; 4]),
                src.pixel(x0, y0 + 1).unwrap_or([0; 4]),
                src.pixel(x0 + 1, y0 + 1).unwrap_or([0; 4]),
            ];
            if p.iter().all(|p| *p == [0; 4]) {
                return None;
            }
            Some(std::array::from_fn(|c| {
                let top = p[0][c] as f32 * (1. - fx) + p[1][c] as f32 * fx;
                let bottom = p[2][c] as f32 * (1. - fx) + p[3][c] as f32 * fx;
                (top * (1. - fy) + bottom * fy).floor().clamp(0., 255.) as u8
            }))
        }
    }
}
impl ImageSurface {
    pub fn draw_image(&mut self, src: ImageView<'_>, draw: FloatDraw) -> Result<()> {
        self.validate()?;
        src.validate()?;
        ensure!(
            draw.destination
                .iter()
                .chain(&draw.source)
                .chain(std::iter::once(&draw.opacity))
                .chain(draw.clip.iter().flatten())
                .chain(draw.quad.iter().flatten().flatten())
                .all(|v| v.is_finite() && v.abs() <= 1_000_000.),
            "invalid float draw parameters"
        );
        let [x, y, w, h] = draw.destination;
        let [sx, sy, sw, sh] = draw.source;
        if w <= 0. || h <= 0. || sw <= 0. || sh <= 0. || draw.opacity <= 0. {
            return Ok(());
        }
        let clip = draw
            .clip
            .unwrap_or([0., 0., self.width as f32, self.height as f32]);
        let (min_x, min_y, max_x, max_y) = if let Some(q) = draw.quad {
            (
                q.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min),
                q.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min),
                q.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max),
                q.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max),
            )
        } else {
            (x, y, x + w, y + h)
        };
        let x0 = min_x.floor().max(0.).max(clip[0]) as i32;
        let y0 = min_y.floor().max(0.).max(clip[1]) as i32;
        let x1 = max_x.ceil().min(self.width as f32).min(clip[0] + clip[2]) as i32;
        let y1 = max_y.ceil().min(self.height as f32).min(clip[1] + clip[3]) as i32;
        if x0 >= x1 || y0 >= y1 {
            return Ok(());
        }
        if draw.quad.is_none()
            && draw.opacity == 1.
            && draw.blend == FloatBlend::Coverage
            && draw.sampling == FloatSampling::NearestClamp
            && w == sw
            && h == sh
            && [x, y, sx, sy, w, h]
                .iter()
                .all(|v| v.abs() <= 32768. && v.fract() == 0.)
        {
            let left = sx as i64 + i64::from(x0) - x as i64;
            let top = sy as i64 + i64::from(y0) - y as i64;
            if left >= 0
                && top >= 0
                && left + i64::from(x1 - x0) <= i64::from(src.width)
                && top + i64::from(y1 - y0) <= i64::from(src.height)
            {
                let row_bytes = (x1 - x0) as usize * 4;
                for yy in y0..y1 {
                    let source_start =
                        (((top + i64::from(yy - y0)) * i64::from(src.width) + left) * 4) as usize;
                    let target_start = ((yy as u32 * self.width + x0 as u32) * 4) as usize;
                    let source_row = &src.rgba[source_start..source_start + row_bytes];
                    let target_row = &mut self.rgba[target_start..target_start + row_bytes];
                    if draw.ignore_source_alpha {
                        target_row.copy_from_slice(source_row);
                        for pixel in target_row.chunks_exact_mut(4) {
                            pixel[3] = 255;
                        }
                    } else if source_row.chunks_exact(4).all(|pixel| pixel[3] == 255) {
                        target_row.copy_from_slice(source_row);
                    } else {
                        for (destination, source) in target_row
                            .chunks_exact_mut(4)
                            .zip(source_row.chunks_exact(4))
                        {
                            blend_pixel(
                                destination.try_into().unwrap(),
                                source.try_into().unwrap(),
                                1.,
                                draw.blend,
                            );
                        }
                    }
                }
                return Ok(());
            }
        }
        let affine = draw.quad.map(|q| {
            let u = [q[3][0] - q[0][0], q[3][1] - q[0][1]];
            let v = [q[1][0] - q[0][0], q[1][1] - q[0][1]];
            let det = u[0] * v[1] - u[1] * v[0];
            (q[0], u, v, det)
        });
        if affine.is_some_and(|(_, _, _, det)| det.abs() <= f32::EPSILON) {
            return Ok(());
        }
        for yy in y0..y1 {
            for xx in x0..x1 {
                let (u, v) = if let Some((origin, axis_u, axis_v, det)) = affine {
                    let px = xx as f32 + 0.5 - origin[0];
                    let py = yy as f32 + 0.5 - origin[1];
                    let u = (px * axis_v[1] - py * axis_v[0]) / det;
                    let v = (axis_u[0] * py - axis_u[1] * px) / det;
                    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                        continue;
                    }
                    (u, v)
                } else {
                    (
                        ((xx as f32 + 0.5 - x) / w).clamp(0., 0.999_999),
                        ((yy as f32 + 0.5 - y) / h).clamp(0., 0.999_999),
                    )
                };
                let shift = if affine.is_some() { 0.5 } else { 0. };
                let Some(mut pixel) =
                    sample(src, sx + u * sw - shift, sy + v * sh - shift, draw.sampling)
                else {
                    continue;
                };
                if draw.ignore_source_alpha {
                    pixel[3] = 255;
                }
                let at = ((yy as u32 * self.width + xx as u32) * 4) as usize;
                blend_pixel(
                    (&mut self.rgba[at..at + 4]).try_into().unwrap(),
                    pixel,
                    draw.opacity,
                    draw.blend,
                );
            }
        }
        Ok(())
    }
    /// Paint outline samples without quantizing coverage before color opacity.
    /// Bold duplicates samples by one pixel; italic offsets each row by 20%.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_coverage(
        &mut self,
        origin: [i32; 2],
        width: u32,
        height: u32,
        coverage: &[f32],
        color: [f32; 4],
        bold: bool,
        italic: bool,
        clip: Option<[f32; 4]>,
    ) -> Result<()> {
        self.validate()?;
        ensure!(
            u64::from(width) * u64::from(height) == coverage.len() as u64
                && coverage.len() <= 4 * 1024 * 1024,
            "invalid glyph coverage"
        );
        ensure!(
            coverage.iter().all(|v| v.is_finite()) && color.iter().all(|v| v.is_finite()),
            "invalid glyph coverage/color"
        );
        ensure!(
            clip.iter()
                .flatten()
                .all(|v| v.is_finite() && v.abs() <= 1_000_000.),
            "invalid glyph clip"
        );
        for row in 0..height {
            for col in 0..width {
                let shift = if italic {
                    ((height as f32 - row as f32).max(0.) * 0.20).round() as i64
                } else {
                    0
                };
                for dx in 0..=u32::from(bold) {
                    let x = i64::from(origin[0]) + i64::from(col) + shift + i64::from(dx);
                    let y = i64::from(origin[1]) + i64::from(row);
                    if x < 0
                        || y < 0
                        || x >= i64::from(self.width)
                        || y >= i64::from(self.height)
                        || clip.is_some_and(|c| {
                            (x as f32) < c[0]
                                || (y as f32) < c[1]
                                || (x as f32) >= c[0] + c[2]
                                || (y as f32) >= c[1] + c[3]
                        })
                    {
                        continue;
                    }
                    let alpha = (coverage[(row * width + col) as usize] * color[3]).clamp(0., 1.);
                    let source = [
                        (color[0].clamp(0., 1.) * 255.).round() as u8,
                        (color[1].clamp(0., 1.) * 255.).round() as u8,
                        (color[2].clamp(0., 1.) * 255.).round() as u8,
                        (alpha * 255.).round() as u8,
                    ];
                    let at = ((y as u32 * self.width + x as u32) * 4) as usize;
                    blend_pixel(
                        (&mut self.rgba[at..at + 4]).try_into().unwrap(),
                        source,
                        1.,
                        FloatBlend::Coverage,
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovered_float_profiles_are_distinct_from_porter_duff() {
        for (mode, expected) in [
            (FloatBlend::Coverage, [55, 50, 45, 160]),
            (FloatBlend::Add, [60, 60, 60, 160]),
            (FloatBlend::Subtract, [0, 0, 0, 160]),
            (FloatBlend::ConstantInterpolation256, [55, 50, 45, 96]),
        ] {
            let mut d = [10, 20, 30, 64];
            blend_pixel(
                &mut d,
                [100, 80, 60, 128],
                if mode == FloatBlend::ConstantInterpolation256 {
                    0.5
                } else {
                    1.
                },
                mode,
            );
            assert_eq!(d, expected);
        }
    }
    #[test]
    fn coverage_keeps_float_opacity_and_bold_sample_order() {
        let mut dst = ImageSurface::filled(3, 1, [0, 0, 0, 255]).unwrap();
        dst.draw_coverage(
            [0, 0],
            2,
            1,
            &[0.5, 0.5],
            [1., 1., 1., 0.5],
            true,
            false,
            None,
        )
        .unwrap();
        assert_eq!(
            dst.rgba,
            vec![64, 64, 64, 255, 112, 112, 112, 255, 64, 64, 64, 255]
        );
    }
    #[test]
    fn invalid_float_draw_does_not_change_the_target() {
        let source = ImageSurface::filled(1, 1, [255; 4]).unwrap();
        let mut target = ImageSurface::filled(1, 1, [0; 4]).unwrap();
        let old = target.clone();
        assert!(target
            .draw_image(
                (&source).into(),
                FloatDraw {
                    destination: [f32::NAN, 0., 1., 1.],
                    source: [0., 0., 1., 1.],
                    opacity: 1.,
                    ignore_source_alpha: false,
                    blend: FloatBlend::Coverage,
                    quad: None,
                    sampling: FloatSampling::NearestClamp,
                    clip: None
                }
            )
            .is_err());
        assert_eq!(target, old);
        assert!(crate::packed::blit(&mut target, 1, &source, 2, 0, 0, 0x38, 0).is_err());
        assert_eq!(target, old);
    }
}
