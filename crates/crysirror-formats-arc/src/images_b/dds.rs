// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned arc_unpacker DDS uses truncated 565 expansion, componentwise DXT1
// transparency, high-nibble-first DXT3 alpha, and padded block dimensions.
use super::util::*;
use anyhow::{ensure, Result};
use crysirror_image::{rgba_len, ImageSurface};
pub fn decode(b: &[u8]) -> Result<ImageSurface> {
    ensure!(b.starts_with(b"DDS "), "invalid DDS signature");
    slice(b, 0, 128)?;
    ensure!(
        u32le(b, 4)? == 124 && u32le(b, 76)? == 32,
        "invalid DDS header sizes"
    );
    ensure!(
        u32le(b, 112)? & (0x200 | 0x200000) == 0 && u32le(b, 24)? <= 1,
        "DDS cubemap/volume needs an explicit multi-surface importer"
    );
    let h = u32le(b, 12)?;
    let w = u32le(b, 16)?;
    rgba_len(w, h)?;
    let flags = u32le(b, 80)?;
    let four = slice(b, 84, 4)?;
    if flags & 4 == 0 {
        ensure!(
            flags & 64 != 0 && u32le(b, 88)? == 32,
            "unsupported DDS pixel format"
        );
        return raw(w, h, slice(b, 128, b.len().saturating_sub(128))?, 4);
    }
    ensure!(
        [b"DXT1".as_slice(), b"DXT3", b"DXT5"].contains(&four),
        "unsupported DDS FOURCC"
    );
    let ow = (w + 3) & !3;
    let oh = (h + 3) & !3;
    let len = rgba_len(ow, oh)?;
    let mut rgba = vec![0; len];
    let block = if four == b"DXT1" { 8 } else { 16 };
    let mut p = 128;
    for by in (0..h as usize).step_by(4) {
        for bx in (0..w as usize).step_by(4) {
            let bytes = slice(b, p, block)?;
            p += block;
            let color_offset = if block == 8 { 0 } else { 8 };
            let cb = &bytes[color_offset..];
            let c0 = c565(u16le(cb, 0)?);
            let c1 = c565(u16le(cb, 2)?);
            let transparent = (0..4).all(|i| c0[i] <= c1[i]);
            let mut colors = [c0, c1, [0; 4], [0; 4]];
            for i in 0..4 {
                if transparent {
                    colors[2][i] = ((u16::from(c0[i]) + u16::from(c1[i])) / 2) as u8;
                } else {
                    colors[2][i] = ((2 * u16::from(c0[i]) + u16::from(c1[i])) / 3) as u8;
                    colors[3][i] = ((2 * u16::from(c1[i]) + u16::from(c0[i])) / 3) as u8;
                }
            }
            let lookup = u32le(cb, 4)?;
            let mut alpha = [0u8; 8];
            if four == b"DXT5" {
                alpha[0] = bytes[0];
                alpha[1] = bytes[1];
                if alpha[0] > alpha[1] {
                    for i in 2..8 {
                        alpha[i] = (((8 - i) * usize::from(alpha[0])
                            + (i - 1) * usize::from(alpha[1]))
                            / 7) as u8;
                    }
                } else {
                    for i in 2..6 {
                        alpha[i] = (((6 - i) * usize::from(alpha[0])
                            + (i - 1) * usize::from(alpha[1]))
                            / 5) as u8;
                    }
                    alpha[7] = 255;
                }
            }
            let mut alook = 0u64;
            if four == b"DXT5" {
                for i in 0..6 {
                    alook |= u64::from(bytes[i + 2]) << (8 * i);
                }
            }
            for i in 0..16 {
                let mut c = colors[((lookup >> (2 * i)) & 3) as usize];
                if four == b"DXT3" {
                    c[3] = if i % 2 == 0 {
                        bytes[i / 2] & 0xf0
                    } else {
                        (bytes[i / 2] & 15) << 4
                    };
                } else if four == b"DXT5" {
                    c[3] = alpha[((alook >> (3 * i)) & 7) as usize];
                }
                let pos = ((by + i / 4) * ow as usize + bx + i % 4) * 4;
                rgba[pos..pos + 4].copy_from_slice(&c);
            }
        }
    }
    Ok(ImageSurface {
        width: ow,
        height: oh,
        rgba,
    })
}
fn c565(c: u16) -> [u8; 4] {
    [
        ((c >> 8) & 248) as u8,
        ((c >> 3) & 252) as u8,
        ((c << 3) & 248) as u8,
        255,
    ]
}
