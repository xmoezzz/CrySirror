// SPDX-License-Identifier: GPL-3.0-or-later
// Adapted from arc_unpacker, Copyright (C) 2016 rr-.
use super::common::*;
use anyhow::{bail, ensure, Result};
use crysirror_image::{ImageDecodeContext, ImageSurface};
pub(super) fn nvsg(b: &[u8]) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 4)?;
    let size = bounded(c.u32()? as usize)?;
    c.skip(10)?;
    let fmt = c.u16()?;
    let w = c.u16()? as u32;
    let mut h = c.u16()? as u32;
    c.skip(8)?;
    let count = c.u32()?;
    c.skip(8)?;
    if fmt == 2 {
        h = h
            .checked_mul(count)
            .ok_or_else(|| anyhow::anyhow!("NVSG frame stack overflow"))?;
    }
    pixels(w, h)?;
    let mut data = zlib(c.rest(), size)?;
    let format = match fmt {
        0 => Format::Bgr,
        1 | 2 => Format::Bgra,
        3 => Format::Gray,
        4 => {
            for v in &mut data {
                if *v != 0 {
                    *v = 255;
                }
            }
            Format::Gray
        }
        _ => bail!("unknown NVSG pixel format"),
    };
    raw(w, h, &data, format)
}
pub(super) fn ykg(b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 6)?;
    ensure!(c.u16()? == 0, "encrypted YKG unsupported by pinned decoder");
    ensure!(c.u32()? == 64, "unsupported YKG header size");
    c.skip(28)?;
    let offset = c.u32()? as usize;
    let size = bounded(c.u32()? as usize)?;
    c.skip(8)?;
    let regions = c.u32()? as usize;
    let region_size = c.u32()? as usize;
    ensure!(region_size % 64 == 0, "truncated YKG region table");
    Cursor::at(b, regions)?.take(region_size)?;
    let mut data = Cursor::at(b, offset)?.take(size)?.to_vec();
    ensure!(
        data.len() >= 8 && &data[1..4] == b"GNP",
        "non-PNG YKG unsupported"
    );
    data[1..4].copy_from_slice(b"PNG");
    ctx.decode_embedded("image.png", &data)
}
pub(super) fn gfb(b: &[u8]) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 12)?;
    let comp = c.u32()? as usize;
    let size = bounded(c.u32()? as usize)?;
    let pos = c.u32()? as usize;
    c.skip(4)?;
    let w = c.u32()?;
    let h = c.u32()?;
    pixels(w, h)?;
    c.skip(2)?;
    let depth = c.u16()?;
    let mut source = Cursor::at(b, pos)?;
    let data = if comp > 0 {
        lzss(source.take(comp)?, size, false)?
    } else {
        source.take(size)?.to_vec()
    };
    let fmt = match depth {
        24 => Format::Bgr,
        32 => {
            if data.chunks_exact(4).any(|p| p[3] != 0) {
                Format::Bgra
            } else {
                Format::Bgrx
            }
        }
        _ => bail!("unsupported GFB bit depth"),
    };
    let mut out = raw(w, h, &data, fmt)?;
    flip(&mut out);
    Ok(out)
}
pub(super) fn gs(b: &[u8]) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 4)?;
    let comp = c.u32()? as usize;
    let size = bounded(c.u32()? as usize)?;
    let pos = c.u32()? as usize;
    c.skip(4)?;
    let w = c.u32()?;
    let h = c.u32()?;
    pixels(w, h)?;
    let depth = c.u32()?;
    let alpha = c.u32()? != 0;
    let data = lzss(Cursor::at(b, pos)?.take(comp)?, size, false)?;
    match depth {
        8 => {
            let mut d = Cursor::new(&data);
            let pal = read_palette(&mut d, 256, 4, true)?;
            palette(w, h, d.rest(), &pal)
        }
        32 => raw(w, h, &data, if alpha { Format::Bgra } else { Format::Bgrx }),
        _ => bail!("unsupported GS bit depth"),
    }
}
pub(super) fn dpng(b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 8)?;
    let count = c.u32()? as usize;
    ensure!(count <= 65536, "DPNG subimage count limit");
    let w = c.u32()?;
    let h = c.u32()?;
    let mut out = blank(w, h)?;
    for _ in 0..count {
        let x = c.u32()?;
        let y = c.u32()?;
        c.skip(8)?;
        let size = bounded(c.u32()? as usize)?;
        c.skip(8)?;
        if size > 0 {
            let image = ctx.decode_embedded("image.png", c.take(size)?)?;
            blit(&mut out, &image, x, y, false);
        }
    }
    Ok(out)
}
pub(super) fn agf(b: &[u8]) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 4)?;
    ensure!(c.u32()? == 1, "unsupported AGF version");
    ensure!(c.u32()? as usize == b.len(), "AGF file size mismatch");
    let pos = c.u32()? as usize;
    c.skip(12)?;
    let w = c.u32()?;
    let h = c.u32()?;
    let n = pixels(w, h)?;
    let mut c = Cursor::at(b, pos)?;
    let mut words = Vec::with_capacity(n);
    while words.len() < n {
        let control = c.u32()?;
        let strategy = control & 255;
        match strategy {
            1 => {
                let count = (control >> 8) as usize;
                ensure!(count > 0 && count <= n - words.len(), "AGF literal size");
                for _ in 0..count {
                    words.push(c.u32()?);
                }
            }
            2 => {
                let count = (control >> 8) as usize;
                ensure!(count > 0 && count <= n - words.len(), "AGF repeated size");
                let v = c.u32()?;
                words.resize(words.len() + count, v);
            }
            3 => {
                let length = (control >> 16) as usize;
                let repeats = ((control >> 8) & 255) as usize;
                ensure!(
                    length > 0 && repeats > 0 && length * repeats <= n - words.len(),
                    "AGF pattern size"
                );
                let pattern = (0..length).map(|_| c.u32()).collect::<Result<Vec<_>>>()?;
                for _ in 0..repeats {
                    words.extend_from_slice(&pattern);
                }
            }
            4 => {
                let count = (control >> 20) as usize;
                let distance = ((control >> 8) & 4095) as usize;
                ensure!(
                    count > 0
                        && count <= n - words.len()
                        && distance > 0
                        && distance <= words.len(),
                    "AGF invalid backreference"
                );
                for _ in 0..count {
                    words.push(words[words.len() - distance]);
                }
            }
            _ => bail!("unknown AGF command"),
        }
    }
    let data = words
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
    raw(w, h, &data, Format::Bgra)
}
pub(super) fn iph(b: &[u8]) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 0x40)?;
    let w = c.u16()? as u32;
    let h = c.u16()? as u32;
    let n = pixels(w, h)?;
    let mut c = Cursor::at(b, 0x50)?;
    ensure!(c.u16()? == 16, "unsupported IPH bit depth");
    let compressed = c.u16()? != 0;
    let mut c = Cursor::at(b, 0x58)?;
    if !compressed {
        return raw(w, h, c.rest(), Format::Bgr555);
    }
    let stride = w as usize;
    let mut words = vec![0u16; n];
    let mut extra = vec![0u8; stride * 2];
    for y in 0..h as usize {
        let row = y * stride;
        if c.u8()? != 0 {
            let mut x = row;
            let mut pixel = 0u16;
            loop {
                let control = c.u8()?;
                if control == 255 {
                    break;
                }
                if control == 254 {
                    let count = c.u8()? as usize + 1;
                    pixel = c.u16()?;
                    ensure!(x + count <= n, "IPH color run outside canvas");
                    for _ in 0..count {
                        words[x] = pixel;
                        x += 1;
                    }
                } else if control < 128 {
                    pixel = (control as u16) << 8 | c.u8()? as u16;
                    ensure!(x < n, "IPH literal outside canvas");
                    words[x] = pixel;
                    x += 1;
                } else {
                    let delta = control & 127;
                    let r = ((pixel & 0x7c00) >> 10) as i32;
                    let g = ((pixel & 0x3e0) >> 5) as i32;
                    let blue = (pixel & 31) as i32;
                    pixel = ((blue + delta as i32 / 25 % 5 - 2)
                        | ((g + delta as i32 / 5 % 5 - 2) << 5)
                        | ((r + delta as i32 % 5 - 2) << 10)) as u16;
                    ensure!(x < n, "IPH delta outside canvas");
                    words[x] = pixel;
                    x += 1;
                }
                if x == n {
                    break;
                }
            }
        } else {
            for x in 0..stride {
                words[row + x] = c.u16()?;
            }
        }
        if c.u8()? != 0 {
            let mut x = 0;
            while x < extra.len() {
                let control = c.u8()?;
                if control == 255 {
                    break;
                }
                let count = if control >= 128 {
                    c.u8()? as usize + 1
                } else {
                    1
                };
                ensure!(count <= extra.len() - x, "IPH extra run outside line");
                extra[x..x + count].fill(control & 127);
                x += count;
            }
            for x in 0..stride {
                if ((32 >> (x % 6)) & extra[x / 6]) != 0 {
                    words[row + x] |= 0x8000;
                }
            }
        } else {
            c.skip(1)?;
        }
    }
    let data = words
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    raw(w, h, &data, Format::Bgr555)
}
pub(super) fn mnc(b: &[u8]) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 10)?;
    let pos = c.u32()? as usize;
    c.skip(4)?;
    let w = c.u32()?;
    let h = c.u32()?;
    pixels(w, h)?;
    c.skip(2)?;
    ensure!(c.u16()? == 24, "unsupported MNC bit depth");
    c.skip(4)?;
    let size = bounded(c.u32()? as usize)?;
    let mut out = raw(w, h, Cursor::at(b, pos)?.take(size)?, Format::Bgr)?;
    flip(&mut out);
    Ok(out)
}
pub(super) fn abm(b: &[u8]) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 18)?;
    let w = c.u32()?;
    let h = c.u32()?;
    let n = pixels(w, h)?;
    c.skip(2)?;
    let depth = c.u16()? as i16;
    ensure!(depth == -8 || depth == 32, "unsupported ABM bit depth");
    let mut c = Cursor::at(b, 54)?;
    let pal = if depth == -8 {
        read_palette(&mut c, 256, 4, true)?
    } else {
        Vec::new()
    };
    let mut data = Vec::new();
    let mut channel = 0;
    let expected = if depth == 32 { n * 4 } else { n };
    while !c.rest().is_empty() {
        let control = c.u8()?;
        let count = if control == 0 || control == 255 {
            c.u8()? as usize
        } else {
            1
        };
        ensure!(count > 0, "zero ABM run");
        for _ in 0..count {
            let v = if control == 0 { 0 } else { c.u8()? };
            ensure!(data.len() < expected, "ABM data exceeds image");
            data.push(v);
            if depth == 32 {
                channel += 1;
                if channel == 3 {
                    ensure!(data.len() < expected, "ABM alpha outside canvas");
                    data.push(control);
                    channel = 0;
                }
            }
        }
    }
    ensure!(data.len() == expected, "truncated ABM pixels");
    if depth == -8 {
        palette(w, h, &data, &pal)
    } else {
        raw(w, h, &data, Format::Bgra)
    }
}
pub(super) fn dbm(b: &[u8]) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 10)?;
    let w = c.u16()? as u32;
    let h = c.u16()? as u32;
    let n = pixels(w, h)?;
    let format = c.u32()?;
    ensure!(c.u16()? == 1, "invalid DBM header");
    let comp = c.u32()? as usize;
    let mut sys = Cursor::new(c.take(comp)?);
    let compressed = sys.u8()? != 0;
    let stored = sys.u32()? as usize;
    let size = bounded(sys.u32()? as usize)?;
    ensure!(stored >= 9, "DBM stored size underflow");
    let data = if compressed {
        lzss(sys.take(stored - 9)?, size, false)?
    } else {
        sys.take(stored - 9)?.to_vec()
    };
    let mut out = if (1..=3).contains(&format) {
        raw(w, h, &data, Format::Bgr)?
    } else if format == 4 {
        ensure!(
            h % 2 == 0 && h >= 2 && data.len() >= n * 3,
            "invalid DBM mask canvas"
        );
        let mut image = raw(w, h / 2, &data[n * 3 / 2..], Format::Bgr)?;
        for (p, m) in image
            .rgba
            .chunks_exact_mut(4)
            .zip(data[..n * 3 / 2].chunks_exact(3))
        {
            p[3] = m[2];
        }
        image
    } else {
        bail!("unsupported DBM format")
    };
    flip(&mut out);
    Ok(out)
}
