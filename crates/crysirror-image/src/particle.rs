//! Exact fixed-point particle-frame sampling and batched sprite planes.
//! This resource operation is independent of a VM, window or VFX scheduler.
use crate::ImageSurface;
use anyhow::{Context, Result, ensure};
use crysirror_core::{image_processing::ImageParticleSprite, state::ClientId};
use std::{collections::BTreeMap, sync::Arc};
/// Bound source copies, cached mip/mixed frames and temporary centered pairs
/// before a worker is admitted. Resource identities never leave this process.
pub fn working_bytes(owner: ClientId, sprites: &[ImageParticleSprite]) -> Result<usize> {
    ensure!(sprites.len() <= 32768, "particle plane sprite budget");
    let mut sources = BTreeMap::new();
    let mut mips = BTreeMap::new();
    let mut mixes = BTreeMap::new();
    let mut scratch = 0usize;
    let identity = |r: &Arc<dyn crysirror_core::image_processing::ImageResource>| {
        Arc::as_ptr(r) as *const () as usize
    };
    for sprite in sprites {
        ensure!(
            sprite.primary.client() == owner
                && sprite
                    .secondary
                    .as_ref()
                    .is_none_or(|s| s.client() == owner),
            "foreign particle image resource"
        );
        ensure!(
            matches!(sprite.primary_format, 1 | 2)
                && matches!(sprite.secondary_format, 1 | 2)
                && sprite.shrink < 32
                && sprite.mix <= 256
                && (0..=256).contains(&sprite.parameter)
                && matches!(sprite.mode,0..=9|0x20..=0x27|0x40|0x41|0x80|0xc0|0xc1|0xf0|0xff),
            "particle sprite profile"
        );
        for (r, format) in std::iter::once((&sprite.primary, sprite.primary_format)).chain(
            sprite
                .secondary
                .as_ref()
                .map(|s| (s, sprite.secondary_format)),
        ) {
            let id = identity(r);
            let size = r.size();
            sources.insert(id, crate::rgba_len(size[0], size[1])?);
            let scaled = size.map(|v| v.wrapping_mul(u32::from(32 - sprite.shrink) * 2048) >> 16);
            let bytes = if scaled[0] == 0 || scaled[1] == 0 {
                0
            } else {
                crate::rgba_len(scaled[0], scaled[1])?
            };
            mips.insert((id, format, sprite.shrink), bytes);
        }
        if let Some(second) = sprite.secondary.as_ref().filter(|_| sprite.mix != 0) {
            let scale = u32::from(32 - sprite.shrink) * 2048;
            let a = sprite.primary.size().map(|n| n.wrapping_mul(scale) >> 16);
            let b = second.size().map(|n| n.wrapping_mul(scale) >> 16);
            if a.iter().chain(&b).all(|v| *v > 0) {
                let bytes = crate::rgba_len(a[0].max(b[0]), a[1].max(b[1]))?;
                mixes.insert(
                    (
                        identity(&sprite.primary),
                        identity(second),
                        sprite.primary_format,
                        sprite.secondary_format,
                        sprite.shrink,
                        sprite.mix,
                    ),
                    bytes,
                );
                scratch = scratch.max(
                    bytes
                        .checked_mul(2)
                        .context("particle temporary pair size overflow")?,
                );
            }
        }
    }
    let cache = sources
        .values()
        .chain(mips.values())
        .chain(mixes.values())
        .try_fold(0usize, |a, b| a.checked_add(*b))
        .context("particle cache byte overflow")?;
    ensure!(
        cache <= 64 * 1024 * 1024,
        "particle source/mip cache budget"
    );
    cache
        .checked_add(scratch)
        .and_then(|v| v.checked_add(sprites.len() * std::mem::size_of::<ImageParticleSprite>()))
        .context("particle worker admission size overflow")
}
pub fn plane(
    owner: ClientId,
    size: [u32; 2],
    sprites: &[ImageParticleSprite],
    check: &dyn Fn() -> Result<()>,
) -> Result<ImageSurface> {
    working_bytes(owner, sprites)?;
    let mut output = ImageSurface {
        width: size[0],
        height: size[1],
        rgba: vec![0; crate::rgba_len(size[0], size[1])?],
    };
    let mut sources: BTreeMap<usize, ImageSurface> = BTreeMap::new();
    let mut mips: BTreeMap<(usize, u8, u8), Option<ImageSurface>> = BTreeMap::new();
    let mut mixed: BTreeMap<(usize, usize, u8, u8, u8, u16), Option<ImageSurface>> =
        BTreeMap::new();
    let identity = |image: &Arc<dyn crysirror_core::image_processing::ImageResource>| {
        Arc::as_ptr(image) as *const () as usize
    };
    let mut cache_bytes = 0usize;
    for sprite in sprites {
        check()?;
        ensure!(
            sprite.primary.client() == owner
                && sprite
                    .secondary
                    .as_ref()
                    .is_none_or(|s| s.client() == owner),
            "foreign particle image resource"
        );
        ensure!(
            matches!(sprite.primary_format, 1 | 2)
                && matches!(sprite.secondary_format, 1 | 2)
                && sprite.shrink < 32
                && sprite.mix <= 256
                && (0..=256).contains(&sprite.parameter),
            "particle sprite profile"
        );
        for (resource, format) in std::iter::once((&sprite.primary, sprite.primary_format)).chain(
            sprite
                .secondary
                .as_ref()
                .map(|s| (s, sprite.secondary_format)),
        ) {
            let id = identity(resource);
            let key = (id, format, sprite.shrink);
            if !sources.contains_key(&id) {
                let size = resource.size();
                let rgba = resource.rgba();
                ensure!(
                    rgba.len() == crate::rgba_len(size[0], size[1])?,
                    "particle source length"
                );
                cache_bytes = cache_bytes
                    .checked_add(rgba.len())
                    .context("particle cache size overflow")?;
                ensure!(
                    cache_bytes <= 64 * 1024 * 1024,
                    "particle source/mip cache budget"
                );
                sources.insert(
                    id,
                    ImageSurface {
                        width: size[0],
                        height: size[1],
                        rgba: rgba.to_vec(),
                    },
                );
            }
            if !mips.contains_key(&key) {
                let image = scale_particle_frame(
                    &sources[&id],
                    i32::from(format),
                    i32::from(sprite.shrink),
                    check,
                )?;
                cache_bytes = cache_bytes
                    .checked_add(image.as_ref().map_or(0, |i| i.rgba.len()))
                    .context("particle mip size overflow")?;
                ensure!(
                    cache_bytes <= 64 * 1024 * 1024,
                    "particle source/mip cache budget"
                );
                mips.insert(key, image);
            }
        }
        let primary = identity(&sprite.primary);
        let key = (primary, sprite.primary_format, sprite.shrink);
        let Some(first) = mips[&key].as_ref() else {
            continue;
        };
        let image = if sprite.mix != 0 && sprite.secondary.is_some() {
            let secondary = identity(sprite.secondary.as_ref().unwrap());
            let mixkey = (
                primary,
                secondary,
                sprite.primary_format,
                sprite.secondary_format,
                sprite.shrink,
                sprite.mix,
            );
            if !mixed.contains_key(&mixkey) {
                let result = if let Some(second) =
                    mips[&(secondary, sprite.secondary_format, sprite.shrink)].as_ref()
                {
                    let size = [
                        first.width.max(second.width),
                        first.height.max(second.height),
                    ];
                    let mut a = ImageSurface {
                        width: size[0],
                        height: size[1],
                        rgba: vec![0; crate::rgba_len(size[0], size[1])?],
                    };
                    let mut b = a.clone();
                    crate::integer_raster::bitmap_blend::blit_mode(
                        &mut a,
                        2,
                        first,
                        2,
                        ((size[0] - first.width) / 2) as i32,
                        ((size[1] - first.height) / 2) as i32,
                        128,
                        0,
                    );
                    crate::integer_raster::bitmap_blend::blit_mode(
                        &mut b,
                        2,
                        second,
                        2,
                        ((size[0] - second.width) / 2) as i32,
                        ((size[1] - second.height) / 2) as i32,
                        128,
                        0,
                    );
                    crate::integer_raster::bitmap_blend::crossfade_cache(
                        &a,
                        &b,
                        i32::from(sprite.primary_format),
                        i32::from(sprite.mix),
                    )
                } else {
                    None
                };
                cache_bytes = cache_bytes
                    .checked_add(result.as_ref().map_or(0, |i| i.rgba.len()))
                    .context("particle mix size overflow")?;
                ensure!(
                    cache_bytes <= 64 * 1024 * 1024,
                    "particle source/mip cache budget"
                );
                mixed.insert(mixkey, result);
            }
            let Some(image) = mixed[&mixkey].as_ref() else {
                continue;
            };
            image
        } else {
            first
        };
        crate::integer_raster::bitmap_blend::blit_mode(
            &mut output,
            2,
            image,
            i32::from(sprite.primary_format),
            sprite.center[0].wrapping_sub(image.width as i32 / 2),
            sprite.center[1].wrapping_sub(image.height as i32 / 2),
            sprite.mode,
            sprite.parameter,
        );
    }
    Ok(output)
}
// 44DCE0 allocates floor(scale*size) but 495880 samples a rounded scaled
// footprint centered inside it. This is not equivalent to nearest resize.
fn scale_particle_frame(
    source: &ImageSurface,
    format: i32,
    shrink: i32,
    check: &dyn Fn() -> Result<()>,
) -> Result<Option<ImageSurface>> {
    source.validate()?;
    ensure!(
        matches!(format, 1 | 2) && (0..32).contains(&shrink),
        "particle frame sampling profile"
    );
    let scale = (32 - shrink) as u32 * 2048;
    let width = source.width.wrapping_mul(scale) >> 16;
    let height = source.height.wrapping_mul(scale) >> 16;
    if width == 0 || height == 0 {
        return Ok(None);
    }
    let w = source.width.wrapping_mul(scale).wrapping_add(32768) >> 16;
    let h = source.height.wrapping_mul(scale).wrapping_add(32768) >> 16;
    let mut out = ImageSurface {
        width,
        height,
        rgba: vec![0; width as usize * height as usize * 4],
    };
    let origin_x = width as i32 / 2 - w as i32 / 2;
    let origin_y = height as i32 / 2 - h as i32 / 2;
    let mut footprint = 65536 / (scale >> 8);
    let kernel = if scale > 32768 && scale < 65536 {
        1
    } else if scale < 32768 {
        2
    } else {
        0
    };
    let (mut start, shift) = (footprint << 7, 8);
    let excluded = if kernel == 0 {
        footprint = 512;
        start = 0;
        footprint >> 9
    } else {
        footprint >> shift
    };
    let span_x = source.width.saturating_sub(excluded) << 16;
    let span_y = source.height.saturating_sub(excluded) << 16;
    let step_x = span_x / w;
    let step_y = span_y / h;
    let pixel = |x: u32, y: u32| -> [u8; 4] {
        if x >= source.width || y >= source.height {
            [0; 4]
        } else {
            source.rgba[((y * source.width + x) * 4) as usize..][..4]
                .try_into()
                .unwrap()
        }
    };
    for y in 0..height {
        check()?;
        for x in 0..width {
            let sx = start.wrapping_add(step_x.wrapping_mul((x as i32 - origin_x) as u32));
            let sy = start.wrapping_add(step_y.wrapping_mul((y as i32 - origin_y) as u32));
            let mut color = [0u8; 4];
            match kernel {
                0 => {
                    let (ix, iy) = (sx >> 16, sy >> 16);
                    let (fx, fy) = ((sx >> 8) & 255, (sy >> 8) & 255);
                    let p = [
                        pixel(ix, iy),
                        pixel(ix + 1, iy),
                        pixel(ix, iy + 1),
                        pixel(ix + 1, iy + 1),
                    ];
                    for c in 0..4 {
                        color[c] = (((256 - fy)
                            * ((256 - fx) * p[0][c] as u32 + fx * p[1][c] as u32)
                            + fy * ((256 - fx) * p[2][c] as u32 + fx * p[3][c] as u32))
                            >> 16) as u8;
                    }
                }
                1 => {
                    let (left, top) = (sx >> 8, sy >> 8);
                    let right = (left + footprint).min(source.width << 8);
                    let bottom = (top + footprint).min(source.height << 8);
                    let area = right
                        .saturating_sub(left)
                        .wrapping_mul(bottom.saturating_sub(top))
                        >> 8;
                    let mut sum = [0u32; 4];
                    let mut py = top;
                    while py < bottom {
                        let ny = ((py + 256) & !255).min(bottom);
                        let mut px = left;
                        while px < right {
                            let nx = ((px + 256) & !255).min(right);
                            let weight = if area == 0 {
                                0
                            } else {
                                ((ny - py) * (nx - px) << 8) / area
                            } >> 8;
                            let p = pixel(px >> 8, py >> 8);
                            for c in 0..4 {
                                sum[c] = sum[c]
                                    .saturating_add((p[c] as u32 * weight) & 65535)
                                    .min(65535);
                            }
                            px = nx;
                        }
                        py = ny;
                    }
                    for c in 0..4 {
                        color[c] = (sum[c] >> 8) as u8;
                    }
                }
                _ => {
                    let (left, top) = (sx >> 16, sy >> 16);
                    let right = (left + ((footprint + 255) >> 8)).min(source.width);
                    let bottom = (top + ((footprint + 255) >> 8)).min(source.height);
                    let mut count = 0;
                    let mut total = 0;
                    let mut sum = [0u32; 4];
                    for py in top..bottom {
                        for px in left..right {
                            let p = pixel(px, py);
                            total += 1;
                            if p[3] != 0 || format == 1 {
                                count += 1;
                                for c in 0..4 {
                                    sum[c] = sum[c].wrapping_add(p[c] as u32);
                                }
                            }
                        }
                    }
                    for c in 0..3 {
                        color[c] = (sum[c] / count.max(1)) as u8;
                    }
                    color[3] = (sum[3] / total.max(1)) as u8;
                }
            }
            if format == 1 {
                color[3] = 255;
            }
            out.rgba[((y * width + x) * 4) as usize..][..4].copy_from_slice(&color);
        }
    }
    Ok(Some(out))
}
