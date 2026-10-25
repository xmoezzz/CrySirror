// SPDX-License-Identifier: GPL-3.0-or-later
// AliceSoft codecs ported from arc_unpacker, Copyright (C) 2016 rr-.
use super::common::*;
use anyhow::{ensure, Result};
use crysirror_image::{ImageDecodeContext, ImageSurface};
pub fn qnt(bytes: &[u8]) -> Result<ImageSurface> {
    let mut c = C::new(bytes);
    ensure!(c.take(4)? == b"QNT\0", "QNT magic");
    ensure!(c.u32()? == 2, "unsupported QNT version");
    c.skip(12)?;
    let (w, h) = (c.u32()?, c.u32()?);
    let depth = c.u32()?;
    ensure!(depth == 24, "unsupported QNT depth");
    c.skip(4)?;
    let (pc, ac) = (c.u32()? as usize, c.u32()? as usize);
    c.skip(24)?;
    let mut out = image(w, h)?;
    let (w, h) = (w as usize, h as usize);
    let ew = (w + 1) & !1;
    let eh = (h + 1) & !1;
    let color = if pc > 0 {
        inflate(c.take(pc)?, ew * eh * 3)?
    } else {
        Vec::new()
    };
    if !color.is_empty() {
        let mut d = C::new(&color);
        for ch in [2, 1, 0] {
            for y in (0..eh).step_by(2) {
                for x in (0..ew).step_by(2) {
                    for (xx, yy) in [(x, y), (x, y + 1), (x + 1, y), (x + 1, y + 1)] {
                        let v = d.u8()?;
                        if xx < w && yy < h {
                            out.rgba[(yy * w + xx) * 4 + ch] = v;
                        }
                    }
                }
            }
        }
    }
    for x in 1..w {
        for ch in 0..3 {
            let i = x * 4 + ch;
            out.rgba[i] = out.rgba[i - 4].wrapping_sub(out.rgba[i]);
        }
    }
    for y in 1..h {
        for ch in 0..3 {
            let i = y * w * 4 + ch;
            out.rgba[i] = out.rgba[i - w * 4].wrapping_sub(out.rgba[i]);
        }
    }
    for y in 1..h {
        for x in 1..w {
            for ch in 0..3 {
                let i = (y * w + x) * 4 + ch;
                let p = (u16::from(out.rgba[i - 4]) + u16::from(out.rgba[i - w * 4])) / 2;
                out.rgba[i] = (p as u8).wrapping_sub(out.rgba[i]);
            }
        }
    }
    if ac > 0 {
        let a = inflate(c.take(ac)?, ew * eh)?;
        let mut a = C::new(&a);
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4 + 3;
                let v = a.u8()?;
                let pred = if y == 0 && x == 0 {
                    None
                } else if y == 0 {
                    Some(out.rgba[i - 4])
                } else if x == 0 {
                    Some(out.rgba[i - w * 4])
                } else {
                    Some(((u16::from(out.rgba[i - 4]) + u16::from(out.rgba[i - w * 4])) / 2) as u8)
                };
                out.rgba[i] = pred.map_or(v, |p| p.wrapping_sub(v));
            }
            if w & 1 != 0 {
                a.skip(1)?;
            }
        }
    } else {
        for p in out.rgba.chunks_exact_mut(4) {
            p[3] = 255;
        }
    }
    Ok(out)
}
pub fn pms8(c: &mut C<'_>, w: usize, h: usize) -> Result<Vec<u8>> {
    let n = w * h;
    let mut o = Vec::with_capacity(n);
    while o.len() < n {
        let b = c.u8()?;
        match b {
            255 | 254 => {
                let k = c.u8()? as usize + 3;
                let d = w * if b == 255 { 1 } else { 2 };
                let count = k.min(n - o.len());
                back(&mut o, d, count, n)?;
            }
            253 => {
                let k = c.u8()? as usize + 4;
                let b = c.u8()?;
                o.resize((o.len() + k).min(n), b);
            }
            252 => {
                let k = c.u8()? as usize + 3;
                let (a, b) = (c.u8()?, c.u8()?);
                for _ in 0..k {
                    for v in [a, b] {
                        if o.len() < n {
                            o.push(v);
                        }
                    }
                }
            }
            248 => o.push(c.u8()?),
            _ => o.push(b),
        }
    }
    Ok(o)
}
fn pms16(c: &mut C<'_>, w: usize, h: usize) -> Result<Vec<u16>> {
    let n = w * h;
    let mut o = Vec::with_capacity(n);
    while o.len() < n {
        let b = c.u8()?;
        match b {
            255 | 254 | 251 | 250 => {
                let (d, k) = match b {
                    255 => (w, c.u8()? as usize + 2),
                    254 => (w * 2, c.u8()? as usize + 2),
                    251 => (w + 1, 1),
                    _ => (
                        w.checked_sub(1)
                            .ok_or_else(|| anyhow::anyhow!("PMS diagonal"))?,
                        1,
                    ),
                };
                ensure!(d > 0 && d <= o.len(), "invalid PMS16 reference");
                for _ in 0..k.min(n - o.len()) {
                    o.push(o[o.len() - d]);
                }
            }
            253 => {
                let k = c.u8()? as usize + 3;
                let v = c.u16()?;
                o.resize((o.len() + k).min(n), v);
            }
            252 => {
                let k = c.u8()? as usize + 2;
                let (a, b) = (c.u16()?, c.u16()?);
                for _ in 0..k {
                    for v in [a, b] {
                        if o.len() < n {
                            o.push(v);
                        }
                    }
                }
            }
            249 => {
                let k = c.u8()? as usize + 1;
                let b = u16::from(c.u8()?);
                let hi = ((b & 0xe0) << 8) | ((b & 0x18) << 6) | ((b & 7) << 2);
                for _ in 0..k.min(n - o.len()) {
                    let b = u16::from(c.u8()?);
                    o.push(hi | ((b & 0xc0) << 5) | ((b & 0x3c) << 3) | (b & 3));
                }
            }
            248 => o.push(c.u16()?),
            _ => o.push(u16::from(b) | (u16::from(c.u8()?) << 8)),
        }
    }
    Ok(o)
}
pub fn pms(bytes: &[u8]) -> Result<ImageSurface> {
    let mut c = C::new(bytes);
    ensure!(c.take(2)? == b"PM", "PMS magic");
    let version = c.u16()?;
    ensure!(matches!(version, 1 | 2), "PMS version");
    c.skip(2)?;
    let depth = c.u16()?;
    c.skip(16)?;
    let (w, h) = (c.u32()?, c.u32()?);
    let (data, extra) = (c.u32()? as usize, c.u32()? as usize);
    let mut out = image(w, h)?;
    let (wi, hi) = (w as usize, h as usize);
    let mut d = C::at(bytes, data)?;
    match depth {
        8 => {
            let pixels = pms8(&mut d, wi, hi)?;
            let mut pal = C::at(bytes, extra)?;
            let mut colors = Vec::new();
            for _ in 0..256 {
                let p = pal.take(3)?;
                colors.push(if version == 1 {
                    [p[0], p[1], p[2], 255]
                } else {
                    [p[2], p[1], p[0], 255]
                });
            }
            out = palette(w, h, &pixels, &colors)?;
        }
        16 => {
            let pix = pms16(&mut d, wi, hi)?;
            for (p, v) in out.rgba.chunks_exact_mut(4).zip(pix) {
                p.copy_from_slice(&[
                    ((v >> 8) & 248) as u8,
                    ((v >> 3) & 252) as u8,
                    ((v << 3) & 248) as u8,
                    255,
                ]);
            }
            if extra != 0 {
                let a = pms8(&mut C::at(bytes, extra)?, wi, hi)?;
                for (p, a) in out.rgba.chunks_exact_mut(4).zip(a) {
                    p[3] = a;
                }
            }
        }
        _ => anyhow::bail!("unsupported PMS bit depth {depth}"),
    }
    Ok(out)
}
pub fn dcf(bytes: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    let mut c = C::new(bytes);
    ensure!(c.take(4)? == b"dcf ", "DCF magic");
    let n = c.u32()? as usize;
    c.skip(n)?;
    ensure!(c.take(4)? == b"dfdl", "DCF dfdl chunk");
    let n = c.u32()? as usize;
    c.skip(n)?;
    ensure!(c.take(4)? == b"dcgd", "DCF dcgd chunk");
    let n = c.u32()? as usize;
    ctx.decode_embedded("embedded.qnt", c.take(n)?)
}
pub fn ajp(bytes: &[u8]) -> Result<ImageSurface> {
    let mut c = C::new(bytes);
    ensure!(c.take(4)? == b"AJP\0", "AJP magic");
    c.skip(8)?;
    let (w, h) = (c.u32()?, c.u32()?);
    let mut out = image(w, h)?;
    let (jo, jn, mo, mn) = (
        c.u32()? as usize,
        c.u32()? as usize,
        c.u32()? as usize,
        c.u32()? as usize,
    );
    let key = [
        0x5d, 0x91, 0xae, 0x87, 0x4a, 0x56, 0x41, 0xcd, 0x83, 0xec, 0x4c, 0x92, 0xb5, 0xcb, 0x16,
        0x34,
    ];
    let decrypt = |at, n| -> Result<Vec<u8>> {
        let mut b = C::at(bytes, at)?.take(n)?.to_vec();
        for (a, k) in b.iter_mut().zip(key) {
            *a ^= k;
        }
        Ok(b)
    };
    let data = decrypt(jo, jn)?;
    let jpeg = crysirror_image::decode_standard(&data)?;
    ensure!(jpeg.width == w && jpeg.height == h, "AJP JPEG dimensions");
    out.rgba = jpeg.rgba;
    if mn != 0 {
        let m = pms(&decrypt(mo, mn)?)?;
        ensure!(m.width == w && m.height == h, "AJP mask dimensions");
        for (p, m) in out.rgba.chunks_exact_mut(4).zip(m.rgba.chunks_exact(4)) {
            p[3] = m[0];
        }
    }
    Ok(out)
}
pub fn vsp(bytes: &[u8]) -> Result<ImageSurface> {
    let mut c = C::new(bytes);
    let (x, y) = (u32::from(c.u16()?), u32::from(c.u16()?));
    let (w, h) = (u32::from(c.u16()?), u32::from(c.u16()?));
    ensure!(w > x && h > y, "invalid VSP rectangle");
    let (mut w, h) = (w - x, h - y);
    let pms = c.u8()? > 0;
    if pms {
        w = (w + 7) / 8 * 8;
        let _ = image(w, h)?;
        let mut p = C::at(bytes, 32)?;
        let mut pal = Vec::new();
        for _ in 0..256 {
            let rgb = p.take(3)?;
            pal.push([rgb[0], rgb[1], rgb[2], 255]);
        }
        let pix = pms8(&mut C::at(bytes, 0x320)?, w as usize, h as usize)?;
        return palette(w, h, &pix, &pal);
    }
    w = w
        .checked_mul(8)
        .ok_or_else(|| anyhow::anyhow!("VSP width overflow"))?;
    let mut out = image(w, h)?;
    let mut p = C::at(bytes, 10)?;
    let mut pal = Vec::new();
    for _ in 0..16 {
        let b = (p.u8()? & 15) * 17;
        let r = (p.u8()? & 15) * 17;
        let g = (p.u8()? & 15) * 17;
        pal.push([r, g, b, 255]);
    }
    let h = h as usize;
    let w = w as usize;
    let mut prev = vec![[0u8; 4]; h];
    let mut cur = prev.clone();
    let mut c = C::at(bytes, 0x3a)?;
    let mut mask = 0;
    for x in 0..w / 8 {
        for plane in 0..4 {
            let mut y = 0;
            while y < h {
                let op = c.u8()?;
                match op {
                    0 => {
                        let n = c.u8()? as usize + 1;
                        for _ in 0..n.min(h - y) {
                            cur[y][plane] = prev[y][plane];
                            y += 1;
                        }
                    }
                    1 => {
                        let n = c.u8()? as usize + 1;
                        let b = c.u8()?;
                        for _ in 0..n.min(h - y) {
                            cur[y][plane] = b;
                            y += 1;
                        }
                    }
                    2 => {
                        let n = c.u8()? as usize + 1;
                        let (a, b) = (c.u8()?, c.u8()?);
                        for _ in 0..n {
                            for v in [a, b] {
                                if y < h {
                                    cur[y][plane] = v;
                                    y += 1;
                                }
                            }
                        }
                    }
                    3..=5 => {
                        let n = c.u8()? as usize + 1;
                        for _ in 0..n.min(h - y) {
                            cur[y][plane] = cur[y][(op - 3) as usize] ^ mask;
                            y += 1;
                        }
                        mask = 0;
                    }
                    6 => mask = 255,
                    7 => {
                        cur[y][plane] = c.u8()?;
                        y += 1;
                    }
                    _ => {
                        cur[y][plane] = op;
                        y += 1;
                    }
                }
            }
        }
        for y in 0..h {
            for bit in 0..8 {
                let mut idx = 0;
                for plane in 0..4 {
                    idx |= ((cur[y][plane] >> (7 - bit)) & 1) << plane;
                }
                out.rgba[(y * w + x * 8 + bit) * 4..][..4].copy_from_slice(&pal[idx as usize]);
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    Ok(out)
}
