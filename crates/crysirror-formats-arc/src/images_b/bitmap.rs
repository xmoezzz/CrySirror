// SPDX-License-Identifier: GPL-3.0-or-later
// Layout/alpha compatibility port of pinned microsoft/bmp and truevision/tga.
use super::util::*;
use anyhow::{bail, ensure, Context, Result};
use crysirror_image::{rgba_len, ImageSurface};
pub fn bmp(b: &[u8]) -> Result<ImageSurface> {
    ensure!(b.starts_with(b"BM"), "invalid BMP signature");
    let offset = u32le(b, 10)? as usize;
    let hs = u32le(b, 14)? as usize;
    ensure!(hs == 12 || hs >= 40, "unsupported BMP header size");
    let head = slice(b, 18, hs.checked_sub(4).context("BMP header overflow")?)?;
    let (w, signed_h, planes, depth, compression, mut pals) = if hs == 12 {
        (
            u32::from(u16le(head, 0)?),
            i32::from(u16le(head, 2)? as i16),
            u16le(head, 4)?,
            u16le(head, 6)?,
            0,
            0,
        )
    } else {
        (
            u32le(head, 0)?,
            u32le(head, 4)? as i32,
            u16le(head, 8)?,
            u16le(head, 10)?,
            u32le(head, 12)?,
            u32le(head, 28)?,
        )
    };
    let h = signed_h.checked_abs().context("invalid BMP height")? as u32;
    let len = rgba_len(w, h)?;
    ensure!(planes == 1, "unsupported BMP plane count");
    ensure!(
        compression == 0 || compression == 3,
        "unsupported BMP compression"
    );
    ensure!(
        [1, 2, 4, 8, 16, 24, 32].contains(&depth),
        "unsupported BMP depth"
    );
    if depth <= 8 && pals == 0 {
        pals = 256;
    }
    if depth > 8 {
        pals = 0;
    }
    ensure!(pals <= 256, "BMP palette too large");
    let mut masks = [0u32; 4];
    let mut palette_pos = 14 + hs;
    let mut rotation = 0;
    if pals == 0 {
        if compression != 3 {
            masks = match depth {
                16 => [0x1f000000, 0xe0030000, 0x007c0000, 0],
                24 => [0xff0000, 0xff00, 0xff, 0],
                32 => [0xff000000, 0xff0000, 0xff00, 0xff],
                _ => bail!("unsupported BMP direct depth"),
            };
        } else {
            let p = if hs == 40 {
                palette_pos += 12;
                14 + hs
            } else {
                ensure!(hs == 52 || hs >= 56, "unsupported BMP masks header");
                54
            };
            masks[2] = u32be(b, p)?;
            masks[1] = u32be(b, p + 4)?;
            masks[0] = u32be(b, p + 8)?;
            if hs >= 56 {
                masks[3] = u32be(b, p + 12)?;
            }
        }
        if depth == 16 {
            for m in &mut masks {
                *m >>= 16;
            }
            ensure!(masks[2] != 0, "BMP red mask is empty");
            while masks[2] & 0x8000 == 0 {
                for m in &mut masks {
                    *m = (*m as u16).rotate_left(1) as u32;
                }
                rotation += 1;
            }
        }
    }
    let mut palette = Vec::new();
    for i in 0..pals as usize {
        let p = slice(b, palette_pos + i * 4, 4)?;
        palette.push([p[2], p[1], p[0], 255]);
    }
    let stride = (u64::from(depth) * u64::from(w)).div_ceil(32) * 4;
    let required = stride
        .checked_mul(u64::from(h))
        .context("BMP stride overflow")?;
    slice(b, offset, usize::try_from(required)?)?;
    let mut rgba = vec![0; len];
    for y in 0..h as usize {
        let row = slice(b, offset + stride as usize * y, stride as usize)?;
        let target_y = if signed_h > 0 { h as usize - 1 - y } else { y };
        let mut bit = 0;
        for x in 0..w as usize {
            let color = if pals > 0 {
                let i = bits(row, &mut bit, depth as usize)? as usize;
                *palette.get(i).context("BMP palette index out of range")?
            } else if depth == 24 && masks == [0xff0000, 0xff00, 0xff, 0] {
                let p = slice(row, x * 3, 3)?;
                [p[2], p[1], p[0], 255]
            } else if depth == 32
                && masks[0] == 0xff000000
                && masks[1] == 0xff0000
                && masks[2] == 0xff00
                && (masks[3] == 0 || masks[3] == 255)
            {
                let p = slice(row, x * 4, 4)?;
                [p[2], p[1], p[0], if masks[3] == 0 { 255 } else { p[3] }]
            } else {
                let mut c = bits(row, &mut bit, depth as usize)?;
                if depth == 16 {
                    c = (c as u16).rotate_left(rotation) as u32;
                }
                let channel = |m: u32| -> u8 {
                    if m == 0 {
                        0
                    } else {
                        ((u64::from(c & m) * 255) / u64::from(m)) as u8
                    }
                };
                [
                    channel(masks[2]),
                    channel(masks[1]),
                    channel(masks[0]),
                    if masks[3] == 0 {
                        255
                    } else {
                        channel(masks[3])
                    },
                ]
            };
            let i = (target_y * w as usize + x) * 4;
            rgba[i..i + 4].copy_from_slice(&color);
        }
    }
    if depth == 32 && rgba.chunks_exact(4).all(|p| p[3] == 0) {
        for p in rgba.chunks_exact_mut(4) {
            p[3] = 255;
        }
    }
    Ok(ImageSurface {
        width: w,
        height: h,
        rgba,
    })
}
pub fn tga(b: &[u8]) -> Result<ImageSurface> {
    let head = slice(b, 0, 18)?;
    let pal = match head[1] {
        0 => false,
        1 => true,
        _ => bail!("unsupported TGA palette mode"),
    };
    let kind = head[2];
    ensure!(
        [1, 2, 3, 9, 10, 11].contains(&kind),
        "unsupported TGA image type"
    );
    let start = u16le(b, 3)?;
    let count = u16le(b, 5)?
        .checked_sub(start)
        .context("invalid TGA palette range")? as usize;
    let pd = head[7];
    let w = u32::from(u16le(b, 12)?);
    let h = u32::from(u16le(b, 14)?);
    let depth = if head[16] == 0 { 32 } else { head[16] };
    let flags = head[17];
    let n = rgba_len(w, h)? / 4;
    // Pinned decoder computes but does not apply interleave flags; preserve its row order.
    let channels = usize::from(depth / 8);
    ensure!([8, 16, 24, 32].contains(&depth), "unsupported TGA depth");
    let mut p = 18 + usize::from(head[0]);
    slice(b, 0, p)?;
    let mut palette = Vec::new();
    if pal {
        ensure!(
            [15, 16, 24, 32].contains(&pd),
            "unsupported TGA palette depth"
        );
        let pc = usize::from(pd.div_ceil(8));
        for _ in 0..count {
            let v = slice(b, p, pc)?;
            p += pc;
            palette.push(match pc {
                2 => color16(u16::from_le_bytes([v[0], v[1]]), false),
                3 => [v[2], v[1], v[0], 255],
                _ => [v[2], v[1], v[0], v[3]],
            });
        }
    }
    let mut data = Vec::with_capacity(n * channels);
    if kind & 8 != 0 {
        while data.len() < n * channels {
            let c = *b.get(p).context("truncated TGA run")?;
            p += 1;
            let count = usize::from(c & 127) + 1;
            ensure!(
                data.len() + count * channels <= n * channels,
                "TGA run exceeds pixel count"
            );
            if c & 128 != 0 {
                let v = slice(b, p, channels)?;
                p += channels;
                for _ in 0..count {
                    data.extend(v);
                }
            } else {
                data.extend(slice(b, p, count * channels)?);
                p += count * channels;
            }
        }
    } else {
        data.extend(slice(b, p, n * channels)?);
    }
    let mut out = vec![0; n * 4];
    let mut bit = 0;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let c = if pal {
                let i = bits(&data, &mut bit, depth as usize)? as usize;
                *palette.get(i).context("TGA palette index out of range")?
            } else {
                let i = (y * w as usize + x) * channels;
                let p = &data[i..i + channels];
                match channels {
                    1 => [p[0], p[0], p[0], 255],
                    2 => color16(u16::from_le_bytes([p[0], p[1]]), true),
                    3 => [p[2], p[1], p[0], 255],
                    _ => [p[2], p[1], p[0], p[3]],
                }
            };
            let tx = if flags & 16 != 0 {
                w as usize - 1 - x
            } else {
                x
            };
            let ty = if flags & 32 == 0 {
                h as usize - 1 - y
            } else {
                y
            };
            let i = (ty * w as usize + tx) * 4;
            out[i..i + 4].copy_from_slice(&c);
        }
    }
    Ok(ImageSurface {
        width: w,
        height: h,
        rgba: out,
    })
}
