//! Stored-byte half-sampling, immutable mip updates and fixed row deformation.
use crate::ImageSurface;
use anyhow::{Result, ensure};
use crysirror_core::image_processing::{ImagePyramidLevel, ImagePyramidResource, ImageResource};
use std::sync::Arc;
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowDeform {
    pub wavelength: u32,
    pub phase: u32,
    pub amplitude: i32,
}
impl RowDeform {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.wavelength != 0, "zero row-deformation wavelength");
        Ok(())
    }
    pub fn output_size(&self, size: [u32; 2]) -> Result<[u32; 2]> {
        self.validate()?;
        let width = size[0].wrapping_add(
            (size[0]
                .wrapping_mul(self.amplitude.wrapping_add(65536) as u32)
                .wrapping_shr(16)
                .wrapping_sub(size[0])
                .wrapping_add(1))
                & !1,
        );
        crate::rgba_len(width, size[1])?;
        Ok([width, size[1]])
    }
}
pub fn select_level(scale: u32) -> u32 {
    if scale == 0 {
        return 4;
    }
    let mut level = 0;
    let mut threshold = 32768;
    while scale <= threshold && level < 4 {
        level += 1;
        threshold >>= 1;
    }
    level
}
fn half(source: &ImageSurface, check: &dyn Fn() -> Result<()>) -> Result<ImageSurface> {
    let size = [source.width.div_ceil(2), source.height.div_ceil(2)];
    let mut out = ImageSurface {
        width: size[0],
        height: size[1],
        rgba: vec![0; crate::rgba_len(size[0], size[1])?],
    };
    for y in 0..out.height {
        check()?;
        for x in 0..out.width {
            let rows = (source.height - y * 2).min(2);
            let cols = (source.width - x * 2).min(2);
            for c in 0..4 {
                let mut sum = 0u16;
                for dy in 0..rows {
                    for dx in 0..cols {
                        sum += u16::from(
                            source.rgba
                                [(((y * 2 + dy) * source.width + x * 2 + dx) * 4) as usize + c],
                        );
                    }
                }
                out.rgba[((y * out.width + x) * 4) as usize + c] =
                    (sum / (rows * cols) as u16) as u8;
            }
        }
    }
    Ok(out)
}
pub fn build(
    source: Arc<dyn ImageResource>,
    check: &dyn Fn() -> Result<()>,
) -> Result<Arc<ImagePyramidResource>> {
    check()?;
    let size = source.size();
    let pixels = source.rgba();
    ensure!(
        pixels.len() == crate::rgba_len(size[0], size[1])?,
        "image pyramid source layout"
    );
    let mut image = ImageSurface {
        width: size[0],
        height: size[1],
        rgba: pixels.to_vec(),
    };
    image.validate()?;
    ensure!(
        u64::from(size[0]) * u64::from(size[1]) <= crate::effects::MAX_EFFECT_PIXELS,
        "image pyramid pixel budget"
    );
    let mut levels = vec![ImagePyramidLevel { size, pixels }];
    while levels.len() < 5 && image.width >= 2 && image.height >= 2 {
        image = half(&image, check)?;
        levels.push(ImagePyramidLevel {
            size: [image.width, image.height],
            pixels: image.rgba.clone().into(),
        });
    }
    let pyramid = Arc::new(ImagePyramidResource {
        client: source.client(),
        levels,
    });
    pyramid.validate(source.client())?;
    check()?;
    Ok(pyramid)
}
pub fn update(
    pyramid: &ImagePyramidResource,
    source: Arc<dyn ImageResource>,
    rect: [i32; 4],
    check: &dyn Fn() -> Result<()>,
) -> Result<Arc<ImagePyramidResource>> {
    pyramid.validate(source.client())?;
    ensure!(
        source.size() == pyramid.levels[0].size,
        "image pyramid update source dimensions"
    );
    let size = source.size();
    let pixels = source.rgba();
    ensure!(
        pixels.len() == crate::rgba_len(size[0], size[1])?,
        "image pyramid source layout"
    );
    let mut image = ImageSurface {
        width: size[0],
        height: size[1],
        rgba: pixels.to_vec(),
    };
    image.validate()?;
    let mut levels = pyramid.levels.clone();
    levels[0].pixels = pixels;
    let [mut left, mut top, width, height] = rect;
    let mut right = left.wrapping_add(width).wrapping_sub(1);
    let mut bottom = top.wrapping_add(height).wrapping_sub(1);
    for level in levels.iter_mut().skip(1) {
        check()?;
        if right.wrapping_sub(left) <= 0 {
            if left < 1 {
                right = right.wrapping_add(1);
            } else {
                left = left.wrapping_sub(1);
            }
        }
        if bottom.wrapping_sub(top) <= 0 {
            if top < 1 {
                bottom = bottom.wrapping_add(1);
            } else {
                top = top.wrapping_sub(1);
            }
        }
        left = (left & !1) >> 1;
        top = (top & !1) >> 1;
        right = (right | 1) >> 1;
        bottom = (bottom | 1) >> 1;
        let fresh = half(&image, check)?;
        let mut pixels = level.pixels.to_vec();
        for y in top.max(0)..=bottom.min(level.size[1] as i32 - 1) {
            check()?;
            for x in left.max(0)..=right.min(level.size[0] as i32 - 1) {
                let i = ((y as u32 * level.size[0] + x as u32) * 4) as usize;
                pixels[i..i + 4].copy_from_slice(&fresh.rgba[i..i + 4]);
            }
        }
        level.pixels = pixels.clone().into();
        image = ImageSurface {
            width: level.size[0],
            height: level.size[1],
            rgba: pixels,
        };
    }
    check()?;
    Ok(Arc::new(ImagePyramidResource {
        client: source.client(),
        levels,
    }))
}

pub fn row_deform(
    source: &ImageSurface,
    spec: &RowDeform,
    check: &dyn Fn() -> Result<()>,
) -> Result<ImageSurface> {
    source.validate()?;
    spec.validate()?;
    let RowDeform {
        wavelength,
        phase,
        amplitude,
    } = *spec;
    if wavelength == 0 {
        anyhow::bail!("row deformation layout");
    }
    let width = source.width.wrapping_add(
        (source
            .width
            .wrapping_mul(amplitude.wrapping_add(65536) as u32)
            .wrapping_shr(16)
            .wrapping_sub(source.width)
            .wrapping_add(1))
            & !1,
    );
    if width == 0 || u64::from(width) * u64::from(source.height) > 16 * 1024 * 1024 {
        anyhow::bail!("row deformation layout");
    }
    let mut out = ImageSurface {
        width,
        height: source.height,
        rgba: vec![0; crate::rgba_len(width, source.height)?],
    };
    let start = source.width.wrapping_sub(width).wrapping_shl(15) as i32;
    for y in 0..source.height {
        check()?;
        let angle =
            f64::from(y.wrapping_add(phase)) * std::f64::consts::TAU / f64::from(wavelength);
        let displacement =
            (angle.sin() * f64::from((amplitude as u32).wrapping_mul(source.width))).trunc();
        // x87 FISTP's out-of-range result is the integer-indefinite value.
        let displacement = if displacement < i32::MIN as f64 || displacement > i32::MAX as f64 {
            i32::MIN
        } else {
            displacement as i32
        };
        let offset = start.wrapping_add(displacement >> 1);
        let mut logical = offset >> 16;
        let weight = 16 - ((offset as u32 >> 12) & 15) as i32;
        let mut cursor = 0u32;
        // 418720 advances the source pointer only for an in-bounds logical
        // index. A positive starting offset still reads the first source byte.
        let mut read = |index: i32| -> [u8; 4] {
            if (index as u32) >= source.width || cursor >= source.width {
                return [0; 4];
            }
            let i = ((y * source.width + cursor) * 4) as usize;
            cursor += 1;
            source.rgba[i..i + 4].try_into().unwrap()
        };
        let mut previous = read(logical);
        for x in 0..width {
            logical = logical.wrapping_add(1);
            let next = read(logical);
            let i = ((y * width + x) * 4) as usize;
            for c in 0..4 {
                out.rgba[i + c] = (i32::from(next[c])
                    + ((i32::from(previous[c]) - i32::from(next[c])) * weight >> 4))
                    .clamp(0, 255) as u8;
            }
            previous = next;
        }
    }
    Ok(out)
}
