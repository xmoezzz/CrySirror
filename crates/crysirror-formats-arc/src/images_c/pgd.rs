// SPDX-License-Identifier: GPL-3.0-or-later
// Adapted from arc_unpacker, Copyright (C) 2016 rr-.
use super::common::*;
use anyhow::{bail, ensure, Result};
use crysirror_image::{ImageDecodeContext, ImageSurface};
fn decompress(input: &[u8], n: usize, c00: bool) -> Result<Vec<u8>> {
    bounded(n)?;
    let mut c = Cursor::new(input);
    let mut out = Vec::with_capacity(n);
    let mut dict = [0u8; 3000];
    let mut write = 0;
    let mut written = -3000i64;
    let mut control = 0u16;
    while out.len() < n {
        control >>= 1;
        if control & 0x100 == 0 {
            control = c.u8()? as u16 | 0xff00;
        }
        if control & 1 != 0 {
            if c00 {
                let mut source = ((if written < 0 { 0 } else { write }) + c.u16()? as usize) % 3000;
                let count = c.u8()? as usize;
                ensure!(count > 0, "zero PGD run");
                for _ in 0..count {
                    let v = dict[source];
                    source = (source + 1) % 3000;
                    ensure!(out.len() < n, "PGD run exceeds output size");
                    out.push(v);
                    dict[write] = v;
                    write = (write + 1) % 3000;
                }
                written += count as i64;
            } else {
                let mut v = c.u16()? as usize;
                let (count, distance) = if v & 8 != 0 {
                    ((v & 7) + 4, v >> 4)
                } else {
                    v = (v << 8) | c.u8()? as usize;
                    (((((v & 0xffc) >> 2) + 1) << 2) | (v & 3), v >> 12)
                };
                ensure!(
                    distance > 0 && distance <= out.len(),
                    "invalid PGD backreference"
                );
                for _ in 0..count.min(n - out.len()) {
                    let v = out[out.len() - distance];
                    out.push(v);
                }
            }
        } else {
            let count = c.u8()? as usize;
            ensure!(count > 0, "zero PGD literal run");
            let bytes = c.take(count)?;
            if c00 {
                ensure!(out.len() + count <= n, "PGD literal exceeds output size");
                for &v in bytes {
                    out.push(v);
                    dict[write] = v;
                    write = (write + 1) % 3000;
                }
                written += count as i64;
            } else {
                let take = count.min(n - out.len());
                out.extend_from_slice(&bytes[..take]);
            }
        }
    }
    Ok(out)
}
pub(super) fn c00(b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 28)?;
    let n = bounded(c.u32()? as usize)?;
    let size = c.u32()? as usize;
    ensure!(size >= 12, "invalid PGD-C00 stored size");
    let data = decompress(c.take(size - 12)?, n, true)?;
    ctx.decode_embedded("embedded.tga", &data)
}
pub(super) fn ge(b: &[u8]) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 12)?;
    let w = c.u32()?;
    let h = c.u32()?;
    let n = pixels(w, h)?;
    c.skip(8)?;
    let filter = c.u16()?;
    c.skip(2)?;
    let size = bounded(c.u32()? as usize)?;
    let comp = c.u32()? as usize;
    let data = decompress(c.take(comp)?, size, false)?;
    if filter == 2 {
        ensure!(
            w % 2 == 0 && h % 2 == 0,
            "PGD chroma subsampling requires even dimensions"
        );
        ensure!(data.len() >= n + n / 2, "truncated PGD chroma planes");
        let block = n / 4;
        let mut bgr = vec![0; n * 3];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let chroma = (y / 2) * (w as usize / 2) + x / 2;
                let a = data[chroma] as i8 as i32;
                let b = data[block + chroma] as i8 as i32;
                let base = (data[block * 2 + y * w as usize + x] as i32) << 7;
                let values = [
                    (base + 226 * a) >> 7,
                    (base - 43 * a - 89 * b) >> 7,
                    (base + 179 * b) >> 7,
                ];
                for j in 0..3 {
                    bgr[(y * w as usize + x) * 3 + j] = values[j].clamp(0, 255) as u8;
                }
            }
        }
        return raw(w, h, &bgr, Format::Bgr);
    }
    if filter == 3 {
        let mut d = Cursor::new(&data);
        d.skip(2)?;
        let depth = d.u16()?;
        let channels = (depth / 8) as usize;
        ensure!(channels == 3 || channels == 4, "unsupported PGD bit depth");
        ensure!(
            d.u16()? as u32 == w && d.u16()? as u32 == h,
            "PGD inner dimensions differ"
        );
        let specs = d.take(h as usize)?.to_vec();
        let mut bytes = d.take(n * channels)?.to_vec();
        let stride = w as usize * channels;
        for y in 0..h as usize {
            let row = y * stride;
            match specs[y] {
                1 => {
                    for x in channels..stride {
                        bytes[row + x] = bytes[row + x - channels].wrapping_sub(bytes[row + x]);
                    }
                }
                2 => {
                    ensure!(y > 0, "PGD first row references missing previous row");
                    for x in 0..stride {
                        bytes[row + x] = bytes[row - stride + x].wrapping_sub(bytes[row + x]);
                    }
                }
                4 => {
                    ensure!(y > 0, "PGD first row references missing previous row");
                    for x in channels..stride {
                        let mean =
                            (bytes[row - stride + x] as u16 + bytes[row + x - channels] as u16) / 2;
                        bytes[row + x] = (mean as u8).wrapping_sub(bytes[row + x]);
                    }
                }
                _ => bail!("unknown PGD delta filter"),
            }
        }
        return raw(
            w,
            h,
            &bytes,
            if channels == 4 {
                Format::Bgra
            } else {
                Format::Bgr
            },
        );
    }
    bail!("unsupported PGD filter {filter}")
}
