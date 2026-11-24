// SPDX-License-Identifier: MPL-2.0
//! `VRAM_DESC` texture descriptors of PAK 0xa79. Pixel data lives after the
//! last RAM page: the descriptor's offset is relative to `file length −
//! header data size` (header word 8). Levels are stored largest first, each
//! face in turn for cube maps.
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::service::AssetReader;
use crysirror_resource::read_exact_at;

/// RSX texture formats used by the installed packages, as stored in the top
/// byte of descriptor word 5 (GCM format without the 0x80 bit and layout
/// flags).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// `CELL_GCM_TEXTURE_A8R8G8B8`, swizzled.
    Argb8,
    /// `CELL_GCM_TEXTURE_COMPRESSED_DXT1`.
    Dxt1,
    /// `CELL_GCM_TEXTURE_COMPRESSED_DXT23`.
    Dxt3,
    /// `CELL_GCM_TEXTURE_COMPRESSED_DXT45`.
    Dxt5,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VramDesc {
    pub name: String,
    /// Texture ID that material samplers refer to (descriptor word 3).
    pub id: u32,
    pub format: Format,
    pub width: u32,
    pub height: u32,
    pub levels: u32,
    /// 6 for cube maps, otherwise 1.
    pub faces: u32,
    /// Byte range relative to the texture base.
    pub offset: u64,
    pub size: u64,
}

fn word(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
}

impl VramDesc {
    /// Parse the 0x34-byte descriptor and its source name at `payload`.
    pub fn read(source: &dyn AssetReader, payload: u64) -> Result<Self> {
        let mut b = [0; 0x34 + 256];
        let avail = (source.len()?.saturating_sub(payload)).min(b.len() as u64) as usize;
        ensure!(avail >= 0x35, "short ND VRAM_DESC");
        read_exact_at(source, payload, &mut b[..avail])?;
        let format = match b[0x14] {
            5 => Format::Argb8,
            6 => Format::Dxt1,
            7 => Format::Dxt3,
            8 => Format::Dxt5,
            f => bail!("ND texture format {f:#x}"),
        };
        let (width, height, levels, faces) = (word(&b, 0x20), word(&b, 0x24), word(&b, 0x1c), word(&b, 0x18));
        ensure!(
            (1..=4096).contains(&width) && (1..=4096).contains(&height) && (1..=13).contains(&levels) && matches!(faces, 1 | 6) && word(&b, 0x28) == 1,
            "ND texture dimensions"
        );
        let end = b[0x34..avail].iter().position(|c| *c == 0).context("unterminated ND texture name")?;
        let name = std::str::from_utf8(&b[0x34..0x34 + end])?.to_owned();
        let d = Self { name, id: word(&b, 0xc), format, width, height, levels, faces, offset: word(&b, 4).into(), size: word(&b, 8).into() };
        ensure!(d.total_bytes() <= d.size, "ND texture size");
        Ok(d)
    }

    fn level_bytes(&self, level: u32) -> u64 {
        let (w, h) = ((self.width >> level).max(1) as u64, (self.height >> level).max(1) as u64);
        match self.format {
            Format::Argb8 => w * h * 4,
            Format::Dxt1 => w.div_ceil(4) * h.div_ceil(4) * 8,
            Format::Dxt3 | Format::Dxt5 => w.div_ceil(4) * h.div_ceil(4) * 16,
        }
    }

    /// Bytes of every level of every face.
    fn total_bytes(&self) -> u64 {
        (0..self.levels).map(|l| self.level_bytes(l)).sum::<u64>() * u64::from(self.faces)
    }

    /// Decode the first level of the first face to RGBA8.
    pub fn decode(&self, source: &dyn AssetReader, texture_base: u64) -> Result<crysirror_image::ImageSurface> {
        let len = self.level_bytes(0) as usize;
        let mut data = vec![0; len];
        let at = texture_base.checked_add(self.offset).context("ND texture offset")?;
        ensure!(at + len as u64 <= source.len()?, "ND texture outside package");
        read_exact_at(source, at, &mut data)?;
        decode_level(self.format, self.width, self.height, &data)
    }
}

/// RSX swizzled (Morton) texel index of (x, y) in a power-of-two surface.
fn swizzle(mut x: u32, mut y: u32, w: u32, h: u32) -> usize {
    let (mut out, mut bit, mut sw, mut sh) = (0usize, 0, w, h);
    while sw > 1 || sh > 1 {
        if sw > 1 {
            out |= ((x & 1) as usize) << bit;
            bit += 1;
            x >>= 1;
            sw >>= 1;
        }
        if sh > 1 {
            out |= ((y & 1) as usize) << bit;
            bit += 1;
            y >>= 1;
            sh >>= 1;
        }
    }
    out
}

pub fn decode_level(format: Format, width: u32, height: u32, data: &[u8]) -> Result<crysirror_image::ImageSurface> {
    use crysirror_image::block_compressed::{Block, decode};
    match format {
        Format::Dxt1 => decode(Block::Bc1, width, height, data),
        Format::Dxt3 => decode(Block::Bc2, width, height, data),
        Format::Dxt5 => decode(Block::Bc3, width, height, data),
        Format::Argb8 => {
            ensure!(width.is_power_of_two() && height.is_power_of_two(), "ND swizzled texture size");
            let n = (width * height) as usize;
            ensure!(data.len() >= n * 4, "short ND texture");
            let mut rgba = vec![0; n * 4];
            for y in 0..height {
                for x in 0..width {
                    let s = swizzle(x, y, width, height) * 4;
                    let d = ((y * width + x) * 4) as usize;
                    rgba[d..d + 4].copy_from_slice(&[data[s + 1], data[s + 2], data[s + 3], data[s]]);
                }
            }
            Ok(crysirror_image::ImageSurface { width, height, rgba })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::service::MemoryAssetReader;

    fn desc(format: u8, w: u32, levels: u32, size: u32) -> Vec<u8> {
        let mut b = vec![0; 0x40];
        for (at, v) in [(4, 0x10), (8, size), (0x18, 1), (0x1c, levels), (0x20, w), (0x24, w), (0x28, 1)] {
            b[at..at + 4].copy_from_slice(&v.to_be_bytes());
        }
        b[0x14] = format;
        b[0x34..0x38].copy_from_slice(b"t.n\0");
        b
    }

    #[test]
    fn descriptor_bounds_and_formats() {
        let d = VramDesc::read(&MemoryAssetReader(desc(6, 8, 2, 40).into()), 0).unwrap();
        assert_eq!((d.format, d.width, d.levels, d.offset, d.name.as_str()), (Format::Dxt1, 8, 2, 0x10, "t.n"));
        // 8×8 DXT1 needs 32 + 8 bytes for two levels.
        assert!(VramDesc::read(&MemoryAssetReader(desc(6, 8, 2, 39).into()), 0).is_err());
        assert!(VramDesc::read(&MemoryAssetReader(desc(9, 8, 1, 64).into()), 0).is_err());
    }

    #[test]
    fn argb_is_morton_ordered() {
        assert_eq!([swizzle(1, 0, 4, 4), swizzle(0, 1, 4, 4), swizzle(2, 0, 4, 4), swizzle(1, 0, 4, 2)], [1, 2, 4, 1]);
        let mut data = vec![0; 16];
        data[4..8].copy_from_slice(&[255, 1, 2, 3]);
        let img = decode_level(Format::Argb8, 2, 2, &data).unwrap();
        assert_eq!(&img.rgba[4..8], &[1, 2, 3, 255]);
    }
}
