// SPDX-License-Identifier: GPL-3.0-or-later
// Adapted from arc_unpacker, Copyright (C) 2016 rr-.
use super::common::*;
use anyhow::{ensure, Context, Result};
use crysirror_image::{ImageDecodeContext, ImageSurface};
fn unpack(input: &[u8], w: u32, h: u32, rct: bool) -> Result<Vec<u8>> {
    let unit = if rct { 3 } else { 1 };
    let n = pixels(w, h)? * unit;
    let mut c = Cursor::new(input);
    let mut out = c.take(unit)?.to_vec();
    let mut shifts = Vec::new();
    if rct {
        for i in 0..6 {
            shifts.push((-1 - i) * 3);
        }
        for row in 1..=3 {
            for i in 0..7 {
                shifts.push((3 - i - w as i64 * row) * 3);
            }
        }
        for i in 0..5 {
            shifts.push((2 - i - w as i64 * 4) * 3);
        }
    } else {
        for i in 0..4 {
            shifts.push(-1 - i);
        }
        for i in 0..7 {
            shifts.push(3 - i - w as i64);
        }
        for i in 0..5 {
            shifts.push(2 - i - w as i64 * 2);
        }
    }
    while out.len() < n {
        let flag = c.u8()?;
        if flag & 0x80 != 0 {
            let (idx, count) = if rct {
                let short = flag & 3;
                (
                    ((flag >> 2) & 31) as usize,
                    if short == 3 {
                        (c.u16()? as usize + 4) * 3
                    } else {
                        short as usize * 3 + 3
                    },
                )
            } else {
                let short = flag & 7;
                (
                    ((flag >> 3) & 15) as usize,
                    if short == 7 {
                        c.u16()? as usize + 10
                    } else {
                        short as usize + 3
                    },
                )
            };
            let source = out.len() as i64 + shifts[idx];
            ensure!(
                source >= 0 && (source as usize) < out.len(),
                "invalid Majiro backreference"
            );
            for j in 0..count.min(n - out.len()) {
                out.push(out[source as usize + j]);
            }
        } else {
            let count = if flag == 0x7f {
                c.u16()? as usize + 128
            } else {
                flag as usize + 1
            };
            let bytes = c.take(count * unit)?;
            let take = bytes.len().min(n - out.len());
            out.extend_from_slice(&bytes[..take]);
        }
    }
    Ok(out)
}
fn decrypt(data: &mut [u8], key: &[u8]) {
    let mut table = [0u32; 256];
    for (i, value) in table.iter_mut().enumerate() {
        let mut poly = i as u32;
        for _ in 0..8 {
            poly = (poly >> 1) ^ if poly & 1 != 0 { 0xedb88320 } else { 0 };
        }
        *value = poly;
    }
    let mut checksum = 0xffffffffu32;
    for &c in key {
        checksum = (checksum >> 8) ^ table[((c as u32 ^ checksum) & 255) as usize];
    }
    checksum ^= 0xffffffff;
    for (i, byte) in data.iter_mut().enumerate() {
        let word = ((i % 1024) / 4) as u32;
        let derived = checksum ^ table[(word.wrapping_add(checksum) & 255) as usize];
        *byte ^= (derived >> ((i % 4) * 8)) as u8;
    }
}
pub(super) fn rc8(b: &[u8]) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 5)?;
    ensure!(
        c.u8()? == b'_' && c.take(2)? == b"00",
        "unsupported RC8 encryption/version"
    );
    let w = c.u32()?;
    let h = c.u32()?;
    pixels(w, h)?;
    c.skip(4)?;
    let pal = read_palette(&mut c, 256, 3, true)?;
    palette(w, h, &unpack(c.rest(), w, h, false)?, &pal)
}
pub(super) fn rct(b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 5)?;
    let flag = c.u8()?;
    let encrypted = flag == b'S';
    ensure!(flag == b'C' || encrypted, "unsupported RCT encryption flag");
    let version = c.take(2)?;
    ensure!(
        version == b"00" || version == b"01",
        "unsupported RCT version"
    );
    let w = c.u32()?;
    let h = c.u32()?;
    pixels(w, h)?;
    let size = c.u32()? as usize;
    let base = if version == b"01" {
        let n = c.u16()? as usize;
        let name = c.take(n)?;
        let name = &name[..name.iter().position(|&x| x == 0).unwrap_or(name.len())];
        let name = std::str::from_utf8(name).context(
            "RCT base filename needs an explicit native-byte resolver; UTF-8 path expected",
        )?;
        if name.is_empty() {
            None
        } else {
            ctx.try_read_relative(&name)?
                .map(|bytes| ctx.decode_embedded(&name, &bytes))
                .transpose()?
        }
    } else {
        None
    };
    let mut compressed = c.take(size)?.to_vec();
    if encrypted {
        ctx.key("majiro/rct")?
            .with_bytes(|key| decrypt(&mut compressed, key));
    }
    let mut overlay = raw(w, h, &unpack(&compressed, w, h, true)?, Format::Bgr)?;
    if version == b"01" {
        for p in overlay.rgba.chunks_exact_mut(4) {
            if p[0] == 255 && p[1] == 0 {
                p[3] = 0;
                p[0] = 0;
            }
        }
    }
    let mut out = blank(w, h)?;
    if let Some(base) = base {
        blit(&mut out, &base, 0, 0, true);
    }
    blit(&mut out, &overlay, 0, 0, true);
    let name = ctx.path.rsplit('/').next().unwrap_or(ctx.path);
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    for extension in ["rc8", "rct"] {
        let mask_name = format!("{stem}_.{extension}");
        if let Some(bytes) = ctx.try_read_relative(&mask_name)? {
            let mask = ctx.decode_embedded(&mask_name, &bytes)?;
            ensure!(
                mask.width == w && mask.height == h,
                "RCT mask dimensions differ"
            );
            for (p, m) in out.rgba.chunks_exact_mut(4).zip(mask.rgba.chunks_exact(4)) {
                p[3] = 255 - m[0];
            }
            break;
        }
    }
    Ok(out)
}
