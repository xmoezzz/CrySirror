// SPDX-License-Identifier: MPL-2.0
//! RealLive `g00` bitmaps as one atlas with pattern regions. Decoder copied
//! from siglus_rs `crates/reallive/src/image.rs` (commit
//! 5f358ef1360b0439e92aed2b749e8f25d8721954, MPL-2.0); only the result type
//! changed (straight RGBA buffer instead of the upstream `Surface`).
//!
//! * type 0: `type:u8 w:u16 h:u16 packed:u32 unpacked:u32` then LZ data
//!   whose literals are 3-byte BGR pixels;
//! * type 1: same header; LZ bytes holding `count:u16`, `count` BGRA
//!   palette entries and one index per pixel;
//! * type 2: `type:u8 w:u16 h:u16 regions:u32` + 24 bytes per region
//!   (`x1 y1 x2 y2 origin_x origin_y`), then `packed:u32 unpacked:u32` and
//!   LZ bytes with, per region, blocks of BGRA pixels. Regions are the
//!   "patterns" RealLive objects select.
use anyhow::{Result, anyhow, bail};

/// A pattern: a sub-rectangle of the bitmap with its own origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
    pub origin_x: i32,
    pub origin_y: i32,
}

impl Region {
    pub fn full(width: i32, height: i32) -> Self {
        Self {
            x1: 0,
            y1: 0,
            x2: width - 1,
            y2: height - 1,
            origin_x: 0,
            origin_y: 0,
        }
    }

    pub fn width(&self) -> i32 {
        (self.x2 - self.x1 + 1).max(0)
    }

    pub fn height(&self) -> i32 {
        (self.y2 - self.y1 + 1).max(0)
    }
}

/// A decoded atlas in straight RGBA, with its pattern regions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Atlas {
    pub width: i32,
    pub height: i32,
    pub rgba: Vec<u8>,
    pub regions: Vec<Region>,
    /// The file carried an alpha channel.
    pub has_alpha: bool,
}

impl Atlas {
    fn new(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            rgba: vec![0; (width.max(0) * height.max(0) * 4) as usize],
            regions: vec![Region::full(width, height)],
            has_alpha: false,
        }
    }
}

/// Atlas size and pattern regions from the header alone (no pixel decode),
/// with the same stacking rule as [`decode_g00`].
pub fn g00_regions(data: &[u8]) -> Result<(i32, i32, Vec<Region>)> {
    let kind = *data.first().ok_or_else(|| anyhow!("g00: empty file"))?;
    let width = i32::from(u16_at(data, 1)?);
    let mut height = i32::from(u16_at(data, 3)?);
    match kind {
        0 | 1 => Ok((width, height, vec![Region::full(width, height)])),
        2 => {
            let count = i32_at(data, 5)?.max(0) as usize;
            if count == 0 || 9 + count * 24 > data.len() {
                bail!("g00: invalid g00 region table");
            }
            let mut regions = Vec::with_capacity(count);
            for i in 0..count {
                let at = 9 + i * 24;
                let mut region = Region {
                    x1: i32_at(data, at)?,
                    y1: i32_at(data, at + 4)?,
                    x2: i32_at(data, at + 8)?,
                    y2: i32_at(data, at + 12)?,
                    origin_x: i32_at(data, at + 16)?,
                    origin_y: i32_at(data, at + 20)?,
                };
                region.x1 = region.x1.clamp(0, width - 1);
                region.x2 = region.x2.clamp(0, width - 1);
                region.y1 = region.y1.clamp(0, height - 1);
                region.y2 = region.y2.clamp(0, height - 1);
                regions.push(region);
            }
            let real: Vec<&Region> = regions
                .iter()
                .filter(|r| r.width() > 0 && r.height() > 0)
                .collect();
            if real.len() > 1 && real.iter().all(|r| *r == real[0]) {
                for (i, region) in regions.iter_mut().enumerate() {
                    region.y1 += i as i32 * height;
                    region.y2 += i as i32 * height;
                }
                height *= count as i32;
            }
            Ok((width, height, regions))
        }
        other => bail!("g00: unsupported g00 type {other}"),
    }
}

fn u16_at(data: &[u8], at: usize) -> Result<u16> {
    data.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| anyhow!("g00: truncated bitmap"))
}

fn i32_at(data: &[u8], at: usize) -> Result<i32> {
    data.get(at..at + 4)
        .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| anyhow!("g00: truncated bitmap"))
}

/// The LZ decoder shared by both g00 variants. `unit` is the literal size
/// (3 for type 0, 1 otherwise) and `min` the smallest copy length in
/// units.
fn lz_extract(src: &[u8], unpacked: usize, unit: usize, min: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(unpacked);
    let mut at = 0;
    while at < src.len() && out.len() < unpacked {
        let flag = src[at];
        at += 1;
        for bit in 0..8 {
            if at >= src.len() || out.len() >= unpacked {
                break;
            }
            if flag & (1 << bit) != 0 {
                let end = (at + unit).min(src.len());
                out.extend_from_slice(&src[at..end]);
                at = end;
            } else {
                if at + 2 > src.len() {
                    at = src.len();
                    break;
                }
                let d = usize::from(u16::from_le_bytes([src[at], src[at + 1]]));
                at += 2;
                let (distance, length) = if unit == 3 {
                    ((d >> 4) * 3, ((d & 0x0f) + 1) * 3)
                } else {
                    (d >> 4, (d & 0x0f) + min)
                };
                if distance == 0 || distance > out.len() {
                    // Corrupt reference: stop rather than read garbage.
                    return out;
                }
                let start = out.len() - distance;
                for i in 0..length {
                    let byte = out[start + i];
                    out.push(byte);
                }
            }
        }
    }
    out.truncate(unpacked);
    out
}

pub fn decode_g00(data: &[u8]) -> Result<Atlas> {
    // CrySirror guard: upstream indexed the header unchecked.
    if data.len() < 9 {
        bail!("g00: truncated header");
    }
    let kind = data[0];
    let width = i32::from(u16_at(data, 1)?);
    let mut height = i32::from(u16_at(data, 3)?);
    if width == 0 || height == 0 || i64::from(width) * i64::from(height) > 64 << 20 {
        bail!("g00: image size {width}x{height}");
    }
    match kind {
        0 | 1 => {
            let unpacked = i32_at(data, 9)?.max(0) as usize;
            let src = data
                .get(13..)
                .ok_or_else(|| anyhow!("g00: truncated g00"))?;
            let mut image = Atlas::new(width, height);
            if kind == 0 {
                let bgr = lz_extract(src, unpacked, 3, 1);
                for (pixel, source) in image
                    .rgba
                    .chunks_exact_mut(4)
                    .zip(bgr.chunks_exact(3))
                {
                    pixel.copy_from_slice(&[source[2], source[1], source[0], 255]);
                }
            } else {
                let bytes = lz_extract(src, unpacked + 1, 1, 2);
                let count = usize::from(u16_at(&bytes, 0)?).min(256);
                let palette: Vec<[u8; 4]> = (0..count)
                    .map(|i| {
                        let at = 2 + i * 4;
                        let b = bytes.get(at..at + 4).unwrap_or(&[0, 0, 0, 255]);
                        [b[2], b[1], b[0], b[3]]
                    })
                    .collect();
                let indices = bytes.get(2 + count * 4..).unwrap_or(&[]);
                let mut has_alpha = false;
                for (pixel, &index) in image.rgba.chunks_exact_mut(4).zip(indices) {
                    let colour = palette
                        .get(usize::from(index))
                        .copied()
                        .unwrap_or([0, 0, 0, 255]);
                    has_alpha |= colour[3] != 255;
                    pixel.copy_from_slice(&colour);
                }
                image.has_alpha = has_alpha;
            }
            Ok(image)
        }
        2 => {
            let count = i32_at(data, 5)?.max(0) as usize;
            if count == 0 || 9 + count * 24 > data.len() {
                bail!("g00: invalid g00 region table");
            }
            let mut regions = Vec::with_capacity(count);
            for i in 0..count {
                let at = 9 + i * 24;
                let mut region = Region {
                    x1: i32_at(data, at)?,
                    y1: i32_at(data, at + 4)?,
                    x2: i32_at(data, at + 8)?,
                    y2: i32_at(data, at + 12)?,
                    origin_x: i32_at(data, at + 16)?,
                    origin_y: i32_at(data, at + 20)?,
                };
                region.x1 = region.x1.clamp(0, width - 1);
                region.x2 = region.x2.clamp(0, width - 1);
                region.y1 = region.y1.clamp(0, height - 1);
                region.y2 = region.y2.clamp(0, height - 1);
                regions.push(region);
            }
            // Newer files stack same-sized regions on top of each other:
            // give each its own band of the canvas.
            let real: Vec<&Region> = regions
                .iter()
                .filter(|r| r.width() > 0 && r.height() > 0)
                .collect();
            let stacked = real.len() > 1 && real.iter().all(|r| *r == real[0]);
            if stacked {
                for (i, region) in regions.iter_mut().enumerate() {
                    region.y1 += i as i32 * height;
                    region.y2 += i as i32 * height;
                }
                height = height
                    .checked_mul(count as i32)
                    .filter(|h| i64::from(*h) * i64::from(width) <= 64 << 20)
                    .ok_or_else(|| anyhow!("g00: stacked atlas too large"))?;
            }
            let head = 9 + count * 24;
            let unpacked = i32_at(data, head + 4)?.max(0) as usize;
            let bytes = lz_extract(data.get(head + 8..).unwrap_or(&[]), unpacked, 1, 2);
            let mut image = Atlas::new(width, height);
            image.has_alpha = true;
            let stored = (i32_at(&bytes, 0).unwrap_or(0).max(0) as usize).min(count);
            for (index, region) in regions.iter().enumerate().take(stored) {
                let offset = i32_at(&bytes, index * 8 + 4)?.max(0) as usize;
                let length = i32_at(&bytes, index * 8 + 8)?.max(0) as usize;
                let mut at = offset + 0x74;
                let end = (offset + length).min(bytes.len());
                while at + 0x5c <= end {
                    let x = i32::from(u16_at(&bytes, at)?) + region.x1;
                    let y = i32::from(u16_at(&bytes, at + 2)?) + region.y1;
                    let w = i32::from(u16_at(&bytes, at + 6)?);
                    let h = i32::from(u16_at(&bytes, at + 8)?);
                    at += 0x5c;
                    for row in 0..h {
                        let dy = y + row;
                        for column in 0..w {
                            let dx = x + column;
                            let source = at + ((row * w + column) * 4) as usize;
                            let Some(px) = bytes.get(source..source + 4) else {
                                break;
                            };
                            if dx < 0 || dy < 0 || dx >= width || dy >= height {
                                continue;
                            }
                            let target = ((dy * width + dx) * 4) as usize;
                            image.rgba[target..target + 4]
                                .copy_from_slice(&[px[2], px[1], px[0], px[3]]);
                        }
                    }
                    at += (w * h * 4) as usize;
                }
            }
            image.regions = regions;
            Ok(image)
        }
        other => bail!("g00: unsupported g00 type {other}"),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn type2(regions: &[[i32; 6]], width: u16, height: u16) -> Vec<u8> {
        let mut out = vec![2];
        out.extend(width.to_le_bytes());
        out.extend(height.to_le_bytes());
        out.extend((regions.len() as i32).to_le_bytes());
        for r in regions {
            for v in r {
                out.extend(v.to_le_bytes());
            }
        }
        // Empty LZ stream: no stored pixels.
        out.extend(0i32.to_le_bytes());
        out.extend(4i32.to_le_bytes());
        out.extend([0xff, 0, 0, 0, 0]);
        out
    }

    #[test]
    fn regions_match_the_decoder() {
        let file = type2(&[[0, 0, 9, 9, 5, 5], [10, 0, 19, 9, 0, 0]], 20, 10);
        let (w, h, regions) = g00_regions(&file).unwrap();
        let atlas = decode_g00(&file).unwrap();
        assert_eq!((w, h), (20, 10));
        assert_eq!(regions, atlas.regions);
        assert_eq!(regions[1].x1, 10);
        assert_eq!(regions[0].origin_x, 5);
    }

    #[test]
    fn identical_regions_are_stacked() {
        let file = type2(&[[0, 0, 9, 9, 0, 0], [0, 0, 9, 9, 0, 0]], 10, 10);
        let (_, h, regions) = g00_regions(&file).unwrap();
        assert_eq!(h, 20);
        assert_eq!((regions[1].y1, regions[1].y2), (10, 19));
        assert_eq!(decode_g00(&file).unwrap().height, 20);
    }
}
