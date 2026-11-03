//! Group filters on premultiplied RGBA, shared by every vector-scene caller.
use super::*;
fn map_straight(
    pixmap: &mut Pixmap,
    transform: impl Fn([f32; 4]) -> [f32; 4],
    check: &dyn Fn() -> Result<()>,
) -> Result<()> {
    for block in pixmap.data_mut().chunks_mut(262144) {
        check()?;
        for p in block.chunks_exact_mut(4) {
            let a = f32::from(p[3]);
            let input = if a == 0. {
                [0., 0., 0., 0.]
            } else {
                [
                    f32::from(p[0]) / a,
                    f32::from(p[1]) / a,
                    f32::from(p[2]) / a,
                    a / 255.,
                ]
            };
            let output = transform(input).map(|n| n.clamp(0., 1.));
            for c in 0..3 {
                p[c] = (output[c] * output[3] * 255.).round() as u8;
            }
            p[3] = (output[3] * 255.).round() as u8;
        }
    }
    Ok(())
}
pub(super) fn color_transform(
    pixmap: &mut Pixmap,
    c: VectorColorTransform,
    check: &dyn Fn() -> Result<()>,
) -> Result<()> {
    map_straight(
        pixmap,
        |p| std::array::from_fn(|i| p[i] * c.multiply[i] + c.add[i]),
        check,
    )
}
fn blur_axis(
    source: &[u8],
    dest: &mut [u8],
    size: [u32; 2],
    radius: usize,
    horizontal: bool,
    check: &dyn Fn() -> Result<()>,
) -> Result<()> {
    if radius == 0 {
        dest.copy_from_slice(source);
        return Ok(());
    }
    let (w, h) = (size[0] as usize, size[1] as usize);
    let (lines, length) = if horizontal { (h, w) } else { (w, h) };
    let offset = |line: usize, i: usize| {
        if horizontal {
            (line * w + i) * 4
        } else {
            (i * w + line) * 4
        }
    };
    let divisor = (radius * 2 + 1) as u32;
    for line in 0..lines {
        check()?;
        let mut sum = [0u32; 4];
        for i in 0..=radius.min(length.saturating_sub(1)) {
            let at = offset(line, i);
            for c in 0..4 {
                sum[c] += u32::from(source[at + c]);
            }
        }
        for i in 0..length {
            let at = offset(line, i);
            for c in 0..4 {
                dest[at + c] = ((sum[c] + divisor / 2) / divisor) as u8;
            }
            if i >= radius {
                let old = offset(line, i - radius);
                for c in 0..4 {
                    sum[c] -= u32::from(source[old + c]);
                }
            }
            if i + radius + 1 < length {
                let new = offset(line, i + radius + 1);
                for c in 0..4 {
                    sum[c] += u32::from(source[new + c]);
                }
            }
        }
    }
    Ok(())
}
fn blur(
    pixmap: &mut Pixmap,
    radius: [u16; 2],
    passes: u8,
    check: &dyn Fn() -> Result<()>,
) -> Result<()> {
    let mut source = pixmap.data().to_vec();
    let mut dest = vec![0; source.len()];
    let size = [pixmap.width(), pixmap.height()];
    for _ in 0..passes {
        blur_axis(&source, &mut dest, size, radius[0] as usize, true, check)?;
        std::mem::swap(&mut source, &mut dest);
        blur_axis(&source, &mut dest, size, radius[1] as usize, false, check)?;
        std::mem::swap(&mut source, &mut dest);
    }
    pixmap.data_mut().copy_from_slice(&source);
    Ok(())
}
fn shadow(
    mut original: Pixmap,
    offset: [i32; 2],
    color: [u8; 4],
    radius: [u16; 2],
    passes: u8,
    strength: f32,
    inner: bool,
    knockout: bool,
    hide_object: bool,
    check: &dyn Fn() -> Result<()>,
) -> Result<Pixmap> {
    let (w, h) = (original.width(), original.height());
    let mut coverage = Pixmap::new(w, h).context("vector shadow coverage allocation")?;
    for (dst, src) in coverage
        .data_mut()
        .chunks_mut(262144)
        .zip(original.data().chunks(262144))
    {
        check()?;
        for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            d[3] = if inner { 255 - s[3] } else { s[3] };
        }
    }
    blur(&mut coverage, radius, passes, check)?;
    let mut result = Pixmap::new(w, h).context("vector shadow allocation")?;
    for y in 0..h as usize {
        check()?;
        for x in 0..w as usize {
            let at = (y * w as usize + x) * 4;
            let sx = x as i32 - offset[0];
            let sy = y as i32 - offset[1];
            let sampled = if sx >= 0 && sy >= 0 && (sx as u32) < w && (sy as u32) < h {
                f32::from(coverage.data()[((sy as u32 * w + sx as u32) * 4 + 3) as usize]) / 255.
            } else if inner {
                1.
            } else {
                0.
            };
            let object = f32::from(original.data()[at + 3]) / 255.;
            let mut alpha = (sampled * strength).clamp(0., 1.) * f32::from(color[3]) / 255.;
            if inner {
                alpha *= object;
            } else if knockout {
                alpha *= 1. - object;
            }
            let p = &mut result.data_mut()[at..at + 4];
            for c in 0..3 {
                p[c] = (f32::from(color[c]) * alpha).round() as u8;
            }
            p[3] = (255. * alpha).round() as u8;
        }
    }
    if !hide_object && !knockout {
        if inner {
            original.draw_pixmap(
                0,
                0,
                result.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            );
            result = original;
        } else {
            result.draw_pixmap(
                0,
                0,
                original.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            );
        }
    }
    Ok(result)
}
pub(super) fn apply(
    mut pixmap: Pixmap,
    filter: &VectorFilter,
    check: &dyn Fn() -> Result<()>,
) -> Result<Pixmap> {
    match filter {
        VectorFilter::BoxBlur { radius, passes } => blur(&mut pixmap, *radius, *passes, check)?,
        VectorFilter::ColorMatrix { matrix: m } => map_straight(
            &mut pixmap,
            |p| {
                std::array::from_fn(|r| {
                    let at = r * 5;
                    m[at] * p[0]
                        + m[at + 1] * p[1]
                        + m[at + 2] * p[2]
                        + m[at + 3] * p[3]
                        + m[at + 4]
                })
            },
            check,
        )?,
        VectorFilter::DropShadow {
            offset,
            color,
            blur,
            passes,
            strength,
            inner,
            knockout,
            hide_object,
        } => {
            return shadow(
                pixmap,
                *offset,
                *color,
                *blur,
                *passes,
                *strength,
                *inner,
                *knockout,
                *hide_object,
                check,
            );
        }
    }
    Ok(pixmap)
}
