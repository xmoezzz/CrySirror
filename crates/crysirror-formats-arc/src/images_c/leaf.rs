// SPDX-License-Identifier: GPL-3.0-or-later
// Adapted from arc_unpacker, Copyright (C) 2016 rr-.
use super::common::*;
use anyhow::{bail, ensure, Result};
use crysirror_image::{ImageDecodeContext, ImageSurface};
pub(super) fn bbm(b: &[u8]) -> Result<ImageSurface> {
    let mut c = Cursor::new(b);
    let w = c.u16()? as u32;
    let h = c.u16()? as u32;
    let mut out = blank(w, h)?;
    let cw = c.u16()? as u32;
    let ch = c.u16()? as u32;
    pixels(cw, ch)?;
    let nx = c.u16()? as u32;
    let ny = c.u16()? as u32;
    let size = bounded(c.u32()? as usize)?;
    ensure!(
        nx > 0
            && ny > 0
            && nx.checked_mul(cw).is_some_and(|x| x <= w)
            && ny.checked_mul(ch).is_some_and(|y| y <= h),
        "BBM tiles outside canvas"
    );
    for y in 0..ny {
        for x in 0..nx {
            let mut tile = Cursor::new(c.take(size)?);
            tile.skip(5)?;
            let count = tile.u16()? as usize;
            tile.skip(11)?;
            let pal = read_palette(&mut tile, count, 4, false)?;
            let image = palette(cw, ch, tile.rest(), &pal)?;
            blit(&mut out, &image, x * cw, y * ch, false);
        }
    }
    Ok(out)
}
pub(super) fn bjr(b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    let image = ctx.decode_embedded("image.bmp", b)?;
    let name = ctx.path.rsplit('/').next().unwrap_or(ctx.path);
    ensure!(
        name.is_ascii(),
        "BJR filename-byte derivation is only verified for ASCII names"
    );
    let bytes = name.as_bytes();
    let mut k1 = 0x10000u32;
    let mut k2 = 0u32;
    let mut k3 = 0u32;
    for &v in bytes {
        k1 = k1.wrapping_sub(v as u32);
        k2 = k2.wrapping_add(v as u32);
        k3 ^= v as u32;
    }
    let mut out = blank(image.width, image.height)?;
    for y in 0..image.height {
        k3 = k3.wrapping_add(7);
        let sy = k3 % image.height;
        for x in 0..image.width {
            let s = ((sy * image.width + x) * 4) as usize;
            let d = ((y * image.width + x) * 4) as usize;
            for channel in 0..3 {
                let inverse = if x & 1 != 0 {
                    channel == 1
                } else {
                    channel != 1
                };
                out.rgba[d + channel] = if inverse {
                    0x100ffu32
                        .wrapping_sub(k1)
                        .wrapping_sub(image.rgba[s + channel] as u32) as u8
                } else {
                    0x10000u32
                        .wrapping_sub(k2)
                        .wrapping_add(image.rgba[s + channel] as u32) as u8
                };
            }
            out.rgba[d + 3] = 255;
        }
    }
    Ok(out)
}
pub(super) fn grp(b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    let mut c = Cursor::new(b);
    let mut w = c.u16()? as u32;
    let mut h = c.u16()? as u32;
    if w == 0 && h == 0 {
        w = c.u16()? as u32;
        h = c.u16()? as u32;
    }
    let n = pixels(w, h)?;
    ensure!(c.rest().len() == n, "GRP indexed size mismatch");
    let indices = c.rest();
    let name = ctx.path.rsplit('/').next().unwrap_or(ctx.path);
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    let mut out = raw(w, h, indices, Format::Gray)?;
    if let Some(bytes) = ctx.try_read_relative(&format!("{stem}.c16"))? {
        let mut p = Cursor::new(&bytes);
        let count = p.u16()? as usize;
        ensure!(count <= 256, "GRP palette count");
        let mut pal = vec![[0, 0, 0, 0]; 256];
        for _ in 0..count {
            let i = p.u8()? as usize;
            let b = p.u8()?;
            let g = p.u8()?;
            let r = p.u8()?;
            pal[i] = [r, g, b, 255];
        }
        out = palette(w, h, indices, &pal)?;
    }
    if let Some(bytes) = ctx.try_read_relative(&format!("{stem}.msk"))? {
        ensure!(bytes.len() >= n, "truncated GRP mask");
        for (p, &m) in out.rgba.chunks_exact_mut(4).zip(&bytes) {
            p[3] = !m;
        }
    }
    Ok(out)
}
pub(super) fn packed(id: &str, b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    if id == "leaf/lfb" {
        let mut c = Cursor::new(b);
        let size = bounded(c.u32()? as usize)?;
        let data = lzss(c.rest(), size, true)?;
        return ctx.decode_embedded("image.bmp", &data);
    }
    let mut c = Cursor::at(b, 12)?;
    let w = c.u16()? as u32;
    let h = c.u16()? as u32;
    let n = pixels(w, h)?;
    let mut out = match id {
        "leaf/lf2" => {
            let mut p = Cursor::at(b, 22)?;
            let count = p.u16()? as usize;
            let pal = read_palette(&mut p, count, 3, true)?;
            palette(w, h, &lzss(p.rest(), n, true)?, &pal)?
        }
        "leaf/lf3" => {
            let pos = c.u32()? as usize;
            raw(
                w,
                h,
                &lzss(Cursor::at(b, pos)?.rest(), n * 2, true)?,
                Format::Bgr555,
            )?
        }
        "leaf/lc3" => {
            let alpha = c.u32()? as usize;
            let color = c.u32()? as usize;
            ensure!(
                alpha <= color && color <= b.len(),
                "invalid LC3 plane offsets"
            );
            let mut image = raw(w, h, &lzss(&b[color..], n * 2, true)?, Format::Bgr555)?;
            let mask = lzss(&b[alpha..color], n, true)?;
            for (p, m) in image.rgba.chunks_exact_mut(4).zip(mask) {
                p[3] = m.wrapping_shl(3);
            }
            image
        }
        _ => bail!("unknown Leaf packed image"),
    };
    flip(&mut out);
    Ok(out)
}
fn permute(v: u8) -> u8 {
    (v & 1)
        | ((v & 4) >> 1)
        | ((v & 16) >> 2)
        | ((v & 64) >> 3)
        | ((v & 2) << 3)
        | ((v & 8) << 2)
        | ((v & 32) << 1)
        | (v & 128)
}
pub(super) fn lfg(b: &[u8]) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 32)?;
    let x1 = c.u16()? as u32;
    let y1 = c.u16()? as u32;
    let x2 = c.u16()? as u32;
    let y2 = c.u16()? as u32;
    ensure!(x2 >= x1 && y2 >= y1, "invalid LFG bounds");
    let w = (x2 - x1 + 1) * 8;
    let h = y2 - y1 + 1;
    let n = pixels(w, h)?;
    let horizontal = c.u8()? != 0;
    let base = c.u8()?;
    c.skip(2)?;
    let stored = bounded(c.u32()? as usize)?;
    ensure!(
        stored
            .checked_mul(2)
            .is_some_and(|size| size >= n && size <= crysirror_image::MAX_DECODED_BYTES),
        "LFG output size mismatch"
    );
    let mut data = vec![base; n];
    let mut ring = [0u8; 4096];
    let mut write = 0xfee;
    let mut count = 0usize;
    let mut control = 0u16;
    while count < n / 2 && !c.rest().is_empty() {
        control = control.wrapping_shl(1);
        if control & 0x80 == 0 {
            control = ((c.u8()? as u16) << 8) | 255;
        }
        let values = if control & 0x8000 != 0 {
            vec![permute(c.u8()?)]
        } else {
            let tmp = c.u16()?;
            let mut read = (tmp >> 4) as usize;
            let mut values = Vec::new();
            for _ in 0..((tmp & 15) as usize + 3).min(n / 2 - count) {
                let v = ring[read];
                read = (read + 1) & 4095;
                values.push(v);
                ring[write] = v;
                write = (write + 1) & 4095;
            }
            values
        };
        let literal = control & 0x8000 != 0;
        for v in values {
            if literal {
                ring[write] = v;
                write = (write + 1) & 4095;
            }
            let index = if horizontal {
                count * 2
            } else {
                (count % h as usize) * w as usize + (count / h as usize) * 2
            };
            data[index] = v >> 4;
            data[index + 1] = v & 15;
            count += 1;
        }
    }
    let mut p = Cursor::at(b, 8)?;
    let mut pal = Vec::new();
    for _ in 0..24 {
        let v = p.u8()?;
        pal.push(v & 0xf0);
        pal.push(v << 4);
    }
    let colors = pal
        .chunks_exact(3)
        .map(|p| [p[0], p[1], p[2], 255])
        .collect::<Vec<_>>();
    palette(w, h, &data, &colors)
}
