// SPDX-License-Identifier: MPL-2.0
//! Bounded HZC1/NVSG image resources with explicit frame/offset metadata.
//! Adapted from rfvp 062fec82033b554b9eb3b5da069518c0c96a354e texture.rs.
//! No VM, files, renderer, font or GPU dependency. Stored colour bytes are
//! retained exactly; BGRA frames may contain associated RGB (not normalized).
use crate::{rgba_len, ImageSurface, MAX_DECODED_BYTES};
use anyhow::{ensure, Context, Result};
#[derive(Debug, Clone)]
pub struct NvsgImage {
    pub format: u16,
    pub size: [u16; 2],
    pub offset: [u16; 2],
    pub uv: [u16; 2],
    pub unknown: [u32; 3],
    pub frames: Vec<Vec<u8>>,
}
fn u16_at(b: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        b.get(at..at + 2)
            .context("NVSG header truncated")?
            .try_into()?,
    ))
}
fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4)
            .context("NVSG header truncated")?
            .try_into()?,
    ))
}
impl NvsgImage {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_DECODED_BYTES, "NVSG encoded byte limit");
        ensure!(
            bytes.get(..4) == Some(b"hzc1") && bytes.get(12..16) == Some(b"NVSG"),
            "NVSG signature"
        );
        let decoded = u32_at(bytes, 4)? as usize;
        let header = u32_at(bytes, 8)? as usize;
        ensure!(header >= 32, "NVSG header length");
        let data_start = 12usize
            .checked_add(header)
            .context("NVSG header overflow")?;
        let format = u16_at(bytes, 18)?;
        let size = [u16_at(bytes, 20)?, u16_at(bytes, 22)?];
        let channels = match format {
            0 => 3,
            1 | 2 => 4,
            3 | 4 => 1,
            _ => anyhow::bail!("unsupported NVSG format {format}"),
        };
        let count = u32_at(bytes, 32)?.max(1) as usize;
        ensure!(count <= 4096, "NVSG frame count limit");
        let rgba = rgba_len(size[0] as u32, size[1] as u32)?;
        ensure!(
            rgba.checked_mul(count)
                .is_some_and(|n| n <= MAX_DECODED_BYTES),
            "NVSG RGBA resident limit"
        );
        let frame_len = usize::from(size[0]) * usize::from(size[1]) * channels;
        ensure!(
            frame_len.checked_mul(count) == Some(decoded) && decoded <= MAX_DECODED_BYTES,
            "NVSG decoded length mismatch/limit"
        );
        let encoded = bytes
            .get(data_start..)
            .context("NVSG payload outside file")?;
        let mut raw = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(encoded, decoded)
            .map_err(|e| anyhow::anyhow!("NVSG inflate: {e:?}"))?;
        ensure!(raw.len() == decoded, "NVSG inflate length mismatch");
        // RFVP's byte coverage profile: 1 means full, other values retain their
        // stored byte. This is distinct from arc_unpacker's nonzero-is-full profile.
        if format == 4 {
            for value in &mut raw {
                if *value == 1 {
                    *value = 255;
                }
            }
        }
        Ok(Self {
            format,
            size,
            offset: [u16_at(bytes, 24)?, u16_at(bytes, 26)?],
            uv: [u16_at(bytes, 28)?, u16_at(bytes, 30)?],
            unknown: [
                u16_at(bytes, 16)? as u32,
                u32_at(bytes, 36)?,
                u32_at(bytes, 40)?,
            ],
            frames: raw.chunks_exact(frame_len).map(<[u8]>::to_vec).collect(),
        })
    }
    /// Materialize one frame without changing its stored alpha association.
    /// Native frontends retain the offset/cut policy; callers choose display blend.
    pub fn frame_stored_rgba(&self, index: usize) -> Result<ImageSurface> {
        let input = self.frames.get(index).context("NVSG frame index")?;
        use crate::pixels::{to_rgba8, PixelLayout};
        let layout = match self.format {
            0 => PixelLayout::Bgr,
            1 | 2 => PixelLayout::Bgra,
            3 | 4 => PixelLayout::Coverage,
            _ => anyhow::bail!("unsupported NVSG format"),
        };
        let rgba = to_rgba8([self.size[0] as u32, self.size[1] as u32], input, layout)?;
        let image = ImageSurface {
            width: self.size[0] as u32,
            height: self.size[1] as u32,
            rgba,
        };
        image.validate()?;
        Ok(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(format: u16, count: u32, pixels: &[u8]) -> Vec<u8> {
        let mut b = vec![0; 44];
        b[..4].copy_from_slice(b"hzc1");
        b[4..8].copy_from_slice(&(pixels.len() as u32).to_le_bytes());
        b[8..12].copy_from_slice(&32u32.to_le_bytes());
        b[12..16].copy_from_slice(b"NVSG");
        b[18..20].copy_from_slice(&format.to_le_bytes());
        b[20..22].copy_from_slice(&2u16.to_le_bytes());
        b[22..24].copy_from_slice(&1u16.to_le_bytes());
        b[24..26].copy_from_slice(&7u16.to_le_bytes());
        b[26..28].copy_from_slice(&9u16.to_le_bytes());
        b[28..30].copy_from_slice(&11u16.to_le_bytes());
        b[30..32].copy_from_slice(&13u16.to_le_bytes());
        b[32..36].copy_from_slice(&count.to_le_bytes());
        b.extend(miniz_oxide::deflate::compress_to_vec_zlib(pixels, 6));
        b
    }
    #[test]
    fn frames_offsets_uvs_and_hidden_channels_remain_independent() {
        let raw = [
            17, 34, 51, 0, 68, 85, 102, 128, 119, 136, 153, 255, 170, 187, 204, 64,
        ];
        let doc = NvsgImage::decode(&fixture(2, 2, &raw)).unwrap();
        assert_eq!(doc.size, [2, 1]);
        assert_eq!(doc.offset, [7, 9]);
        assert_eq!(doc.uv, [11, 13]);
        assert_eq!(doc.frames, vec![raw[..8].to_vec(), raw[8..].to_vec()]);
        assert_eq!(
            doc.frame_stored_rgba(0).unwrap().rgba,
            [51, 34, 17, 0, 102, 85, 68, 128]
        );
        assert!(doc.frame_stored_rgba(2).is_err());
    }
    #[test]
    fn zero_count_and_every_native_pixel_mode() {
        for (format, raw, expected) in [
            (0, vec![1, 2, 3, 4, 5, 6], vec![3, 2, 1, 255, 6, 5, 4, 255]),
            (
                1,
                vec![1, 2, 3, 4, 5, 6, 7, 8],
                vec![3, 2, 1, 4, 7, 6, 5, 8],
            ),
            (3, vec![0, 128], vec![255, 255, 255, 0, 255, 255, 255, 128]),
            (4, vec![1, 2], vec![255, 255, 255, 255, 255, 255, 255, 2]),
        ] {
            let d = NvsgImage::decode(&fixture(format, 0, &raw)).unwrap();
            assert_eq!(d.frames.len(), 1);
            assert_eq!(d.frame_stored_rgba(0).unwrap().rgba, expected);
        }
    }
    #[test]
    fn malformed_lengths_headers_and_inflate_budgets_fail() {
        let good = fixture(1, 1, &[0; 8]);
        for n in 0..44 {
            assert!(NvsgImage::decode(&good[..n]).is_err(), "length {n}");
        }
        for (at, value) in [(4, u32::MAX), (8, u32::MAX), (32, 4097)] {
            let mut b = good.clone();
            b[at..at + 4].copy_from_slice(&value.to_le_bytes());
            assert!(NvsgImage::decode(&b).is_err());
        }
        assert!(NvsgImage::decode(&fixture(1, 1, &[0; 1024])).is_err());
    }
    #[test]
    fn associated_tone_keeps_alpha_and_identity_bytes() {
        let mut pixels = [80, 100, 120, 128, 10, 20, 30, 0];
        crate::associated::tone_channels_percent(&mut pixels, [50, 100, 200]).unwrap();
        assert_eq!(pixels, [40, 100, 128, 128, 0, 0, 0, 0]);
        let mut hidden = [30, 40, 50, 0];
        crate::associated::tone_channels_percent(&mut hidden, [100; 3]).unwrap();
        assert_eq!(hidden, [30, 40, 50, 0]);
        assert!(crate::associated::tone_channels_percent(&mut [0; 3], [100; 3]).is_err());
    }
}
