// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from arc_unpacker PRT/SOTES decoders, Copyright (C) 2016 rr-.
use super::common::*;
use anyhow::{ensure, Result};
use crysirror_image::ImageSurface;
pub fn prt(bytes: &[u8]) -> Result<ImageSurface> {
    let mut c = C::new(bytes);
    ensure!(c.take(4)? == b"PRT\0", "PRT magic");
    let version = c.u16()?;
    ensure!(matches!(version, 0x65 | 0x66), "unsupported PRT version");
    let depth = c.u16()?;
    ensure!(matches!(depth, 8 | 24 | 32), "unsupported PRT depth");
    let (pal, at) = (c.u16()? as usize, c.u16()? as usize);
    let (mut w, mut h) = (u32::from(c.u16()?), u32::from(c.u16()?));
    let alpha = if version == 0x66 {
        let a = c.u32()? != 0;
        c.skip(8)?;
        let (w2, h2) = (c.u32()?, c.u32()?);
        if w2 != 0 {
            w = u32::from(w2 as u16);
        }
        if h2 != 0 {
            h = u32::from(h2 as u16);
        }
        a
    } else {
        false
    };
    let mut out = image(w, h)?;
    let ch = depth as usize / 8;
    let stride = (w as usize * ch).div_ceil(4) * 4;
    let mut palc = Vec::new();
    if depth == 8 {
        let mut p = C::at(bytes, pal)?;
        for _ in 0..256 {
            let q = p.take(4)?;
            palc.push([q[2], q[1], q[0], 255]);
        }
    }
    let mut c = C::at(bytes, at)?;
    for y in 0..h as usize {
        let row = c.take(stride)?;
        for x in 0..w as usize {
            let src = &row[x * ch..];
            let color = if depth == 8 {
                palc[src[0] as usize]
            } else {
                [
                    src[2],
                    src[1],
                    src[0],
                    if depth == 32 { src[3] } else { 255 },
                ]
            };
            let i = ((h as usize - 1 - y) * w as usize + x) * 4;
            out.rgba[i..i + 4].copy_from_slice(&color);
        }
    }
    if alpha {
        for p in out.rgba.chunks_exact_mut(4) {
            p[3] = c.u8()?;
        }
    }
    Ok(out)
}
fn at32(bytes: &[u8], at: usize) -> Result<u32> {
    C::at(bytes, at)?.u32()
}
pub fn sotes_probe(bytes: &[u8]) -> bool {
    if bytes.len() < 0x454 {
        return false;
    }
    let base = at32(bytes, 0x448).unwrap();
    at32(bytes, 0x438).unwrap().wrapping_sub(base) == 0x2711
        && at32(bytes, 0x450).unwrap().wrapping_sub(base) <= 0x80
}
pub fn sotes(bytes: &[u8]) -> Result<ImageSurface> {
    ensure!(sotes_probe(bytes), "SOTES header");
    let base = at32(bytes, 0x448)?;
    let offset = 0x458usize + at32(bytes, 0x450)?.wrapping_sub(base) as usize;
    let depth = u32::from(C::at(bytes, 0x430)?.u16()?).wrapping_sub(base);
    ensure!(matches!(depth, 8 | 24), "unsupported SOTES depth");
    let t = at32(bytes, 0x440)?.wrapping_sub(base);
    let w = at32(bytes, 4 + 4 * t as usize)?.wrapping_sub(base);
    let t = at32(bytes, 0x18)?.wrapping_sub(base);
    let h = at32(bytes, 0x420 + 4 * t as usize)?.wrapping_sub(base);
    let count = size(w, h, depth as usize / 8)?;
    let data = C::at(bytes, offset)?.take(count)?;
    if depth == 24 {
        return bgr(w, h, data, 3, true);
    }
    let mut c = C::at(bytes, 0x20)?;
    let mut p = Vec::new();
    for _ in 0..256 {
        let q = c.take(4)?;
        p.push([q[2], q[1], q[0], 255]);
    }
    let mut out = palette(w, h, data, &p)?;
    let stride = w as usize * 4;
    for y in 0..h as usize / 2 {
        for x in 0..stride {
            out.rgba
                .swap(y * stride + x, (h as usize - 1 - y) * stride + x);
        }
    }
    Ok(out)
}
