// SPDX-License-Identifier: MPL-2.0
//! Deterministic, bounded PNG serialization for decoded resource frames. Using
//! stored DEFLATE blocks gives a predictable size before decoding an archive
//! entry, without maintaining private image exporters in format plugins.
use crate::{rgba_len, ImageSurface};
use anyhow::{ensure, Result};
const fn crc_table() -> [u32; 256] {
    let mut table = [0; 256];
    let mut i = 0;
    while i < 256 {
        let mut v = i as u32;
        let mut k = 0;
        while k < 8 {
            v = if v & 1 != 0 {
                (v >> 1) ^ 0xedb88320
            } else {
                v >> 1
            };
            k += 1;
        }
        table[i] = v;
        i += 1;
    }
    table
}
const CRC: [u32; 256] = crc_table();
fn crc(data: &[u8]) -> u32 {
    let mut v = !0;
    for b in data {
        v = CRC[((v ^ u32::from(*b)) & 255) as usize] ^ (v >> 8);
    }
    !v
}
pub fn png_uncompressed_len(width: u32, height: u32) -> Result<usize> {
    let raw = rgba_len(width, height)? + height as usize;
    Ok(raw + raw.div_ceil(65535) * 5 + 63)
}
fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let begin = out.len();
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let sum = crc(&out[begin..]);
    out.extend_from_slice(&sum.to_be_bytes());
}
pub fn encode_png_uncompressed(image: &ImageSurface) -> Result<Vec<u8>> {
    image.validate()?;
    let expected = png_uncompressed_len(image.width, image.height)?;
    let stride = image.width as usize * 4 + 1;
    let raw = (stride as u64 * u64::from(image.height)) as usize;
    let mut out = Vec::with_capacity(expected);
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = image.width.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&image.height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    let payload_len = raw + raw.div_ceil(65535) * 5 + 6;
    out.extend_from_slice(&(payload_len as u32).to_be_bytes());
    let tag_start = out.len();
    out.extend_from_slice(b"IDAT");
    out.extend_from_slice(&[0x78, 0x01]);
    let (mut at, mut s1, mut s2) = (0usize, 1u32, 0u32);
    while at < raw {
        let count = (raw - at).min(65535);
        out.push(u8::from(at + count == raw));
        out.extend_from_slice(&(count as u16).to_le_bytes());
        out.extend_from_slice(&(!(count as u16)).to_le_bytes());
        let begin = out.len();
        let end = at + count;
        while at < end {
            let col = at % stride;
            if col == 0 {
                out.push(0);
                at += 1;
            } else {
                let n = (stride - col).min(end - at);
                let offset = (at / stride) * (stride - 1) + col - 1;
                out.extend_from_slice(&image.rgba[offset..offset + n]);
                at += n;
            }
        }
        for block in out[begin..].chunks(5552) {
            for b in block {
                s1 += u32::from(*b);
                s2 += s1;
            }
            s1 %= 65521;
            s2 %= 65521;
        }
    }
    out.extend_from_slice(&((s2 << 16) | s1).to_be_bytes());
    let sum = crc(&out[tag_start..]);
    out.extend_from_slice(&sum.to_be_bytes());
    chunk(&mut out, b"IEND", &[]);
    ensure!(out.len() == expected, "PNG serializer size invariant");
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_size_and_pixels_survive_block_boundaries() {
        for (width, height) in [(1, 1), (257, 80), (1024, 20)] {
            let image = ImageSurface {
                width,
                height,
                rgba: (0..rgba_len(width, height).unwrap())
                    .map(|i| i.wrapping_mul(37) as u8)
                    .collect(),
            };
            let encoded = encode_png_uncompressed(&image).unwrap();
            assert_eq!(encoded.len(), png_uncompressed_len(width, height).unwrap());
            assert_eq!(crate::decode_standard(&encoded).unwrap(), image);
        }
        assert!(png_uncompressed_len(0, 4).is_err());
        assert!(png_uncompressed_len(8192, 8192).is_err());
    }
}
