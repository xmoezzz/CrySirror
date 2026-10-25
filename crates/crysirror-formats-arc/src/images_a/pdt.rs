// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from arc_unpacker RealLive image decoders, Copyright (C) 2016 rr-.
use super::common::*;
use anyhow::{ensure, Result};
use crysirror_image::ImageSurface;
pub fn pdt9(bytes: &[u8]) -> Result<ImageSurface> {
    let mut c = C::new(bytes);
    ensure!(c.u8()? == b'9', "PDT9 signature");
    c.skip(2)?;
    let (x, y) = (u32::from(c.u16()?), u32::from(c.u16()?));
    let (ex, ey) = (u32::from(c.u16()?) + 1, u32::from(c.u16()?) + 1);
    ensure!(ex > x && ey > y, "PDT9 rectangle");
    let (w, h) = (ex - x, ey - y);
    let _ = image(w, h)?;
    let mut pal = Vec::new();
    for _ in 0..256 {
        let p = c.take(4)?;
        pal.push([p[2], p[1], p[0], 255]);
    }
    let mut distances = Vec::new();
    for _ in 0..16 {
        distances.push(c.u16()? as i16);
    }
    let n = w as usize * h as usize;
    let mut pix = vec![0; w as usize];
    while pix.len() < n + w as usize {
        let flag = c.u8()?;
        for bit in (0..8).rev() {
            if pix.len() >= n + w as usize {
                break;
            }
            if flag & (1 << bit) != 0 {
                let t = c.u8()?;
                let d = distances[(t >> 4) as usize];
                let count = ((t & 15) as usize + 2).min(n + w as usize - pix.len());
                for _ in 0..count {
                    let v = if d > 0 && d as usize <= pix.len() {
                        pix[pix.len() - d as usize]
                    } else {
                        0
                    };
                    pix.push(v);
                }
            } else {
                pix.push(c.u8()?);
            }
        }
    }
    palette(w, h, &pix[w as usize..], &pal)
}
fn lz(
    bytes: &[u8],
    size: usize,
    unit: usize,
    delta: usize,
    msb: bool,
    pdt: bool,
) -> Result<Vec<u8>> {
    ensure!(size <= crysirror_image::MAX_DECODED_BYTES, "LZ image limit");
    let mut c = C::new(bytes);
    let mut o = Vec::with_capacity(size);
    while o.len() < size {
        let flag = c.u8()?;
        for i in 0..8 {
            if o.len() >= size {
                break;
            }
            let bit = if msb { 7 - i } else { i };
            if flag & (1 << bit) != 0 {
                o.extend_from_slice(c.take(unit)?);
            } else {
                let t = c.u16()? as usize;
                let count = ((t & 15) + delta) * unit;
                let distance = ((t >> 4) + usize::from(pdt)) * unit;
                let count = count.min(size - o.len());
                back(&mut o, distance, count, size)?;
            }
        }
    }
    ensure!(o.len() == size, "invalid image output size");
    Ok(o)
}
pub fn pdt10(bytes: &[u8]) -> Result<ImageSurface> {
    let mut c = C::new(bytes);
    ensure!(c.take(8)? == b"PDT10\0\0\0", "PDT10 magic");
    c.skip(4)?;
    let (w, h) = (c.u32()?, c.u32()?);
    c.skip(8)?;
    let mask = c.u32()? as usize;
    let _ = image(w, h)?;
    let end = if mask != 0 { mask } else { bytes.len() };
    ensure!(end >= 32 && end <= bytes.len(), "PDT10 mask offset");
    let color = lz(
        &bytes[32..end],
        w as usize * h as usize * 3,
        3,
        1,
        true,
        true,
    )?;
    let mut out = bgr(w, h, &color, 3, false)?;
    if mask != 0 {
        let mut c = C::at(bytes, mask)?;
        let n = w as usize * h as usize;
        let mut a = Vec::with_capacity(n);
        while a.len() < n {
            let f = c.u8()?;
            for i in (0..8).rev() {
                if a.len() >= n {
                    break;
                }
                if f & (1 << i) != 0 {
                    a.push(c.u8()?);
                } else {
                    let len = c.u8()? as usize + 2;
                    let d = c.u8()? as usize + 1;
                    let take = len.min(n - a.len());
                    back(&mut a, d, take, n)?;
                }
            }
        }
        for (p, a) in out.rgba.chunks_exact_mut(4).zip(a) {
            p[3] = a;
        }
    }
    Ok(out)
}
pub fn g00(bytes: &[u8]) -> Result<ImageSurface> {
    let mut c = C::new(bytes);
    let v = c.u8()?;
    let (w, h) = (u32::from(c.u16()?), u32::from(c.u16()?));
    let mut out = image(w, h)?;
    let mut regions = Vec::new();
    if v == 2 {
        let n = c.u32()?;
        ensure!(n <= 65536, "G00 region limit");
        for _ in 0..n {
            let (x, y) = (c.u32()?, c.u32()?);
            c.skip(16)?;
            regions.push((x, y));
        }
    }
    ensure!(v <= 2, "unsupported G00 version");
    let packed = c.u32()? as usize;
    let size = c.u32()? as usize;
    ensure!(packed >= 8, "G00 compressed size");
    let size = if v == 0 {
        ensure!(
            size >= w as usize * h as usize * 3,
            "G00 short declared size"
        );
        w as usize * h as usize * 3
    } else {
        size
    };
    let data = lz(
        c.take(packed - 8)?,
        size,
        if v == 0 { 3 } else { 1 },
        if v == 0 { 1 } else { 2 },
        false,
        false,
    )?;
    if v == 0 {
        return bgr(w, h, &data, 3, false);
    }
    let mut d = C::new(&data);
    if v == 1 {
        let n = d.u16()?;
        let mut p = Vec::new();
        for _ in 0..n {
            let q = d.take(4)?;
            p.push([q[2], q[1], q[0], q[3]]);
        }
        return palette(w, h, d.take(w as usize * h as usize)?, &p);
    }
    ensure!(
        d.u32()? as usize == regions.len(),
        "G00 region count mismatch"
    );
    let mut blocks = Vec::new();
    for _ in &regions {
        blocks.push((d.u32()? as usize, d.u32()? as usize));
    }
    for ((x, y), (at, size)) in regions.into_iter().zip(blocks) {
        if size == 0 {
            continue;
        }
        let mut b = C::new(C::at(&data, at)?.take(size)?);
        ensure!(b.u16()? == 1, "G00 block type");
        let n = b.u16()?;
        b.skip(0x70)?;
        for _ in 0..n {
            let (px, py) = (u32::from(b.u16()?), u32::from(b.u16()?));
            b.skip(2)?;
            let (pw, ph) = (u32::from(b.u16()?), u32::from(b.u16()?));
            b.skip(0x52)?;
            let x = x
                .checked_add(px)
                .ok_or_else(|| anyhow::anyhow!("G00 coordinate overflow"))?;
            let y = y
                .checked_add(py)
                .ok_or_else(|| anyhow::anyhow!("G00 coordinate overflow"))?;
            ensure!(
                x.checked_add(pw).is_some_and(|e| e <= w)
                    && y.checked_add(ph).is_some_and(|e| e <= h),
                "G00 part outside canvas"
            );
            let part = bgr(pw, ph, b.take(pw as usize * ph as usize * 4)?, 4, false)?;
            for yy in 0..ph as usize {
                let dest = ((yy + y as usize) * w as usize + x as usize) * 4;
                let src = yy * pw as usize * 4;
                out.rgba[dest..dest + pw as usize * 4]
                    .copy_from_slice(&part.rgba[src..src + pw as usize * 4]);
            }
        }
    }
    Ok(out)
}
