// SPDX-License-Identifier: GPL-3.0-or-later
// Adapted from arc_unpacker, Copyright (C) 2016 rr-.
use super::common::*;
use anyhow::{bail, ensure, Result};
use crysirror_image::ImageSurface;
fn copy(out: &mut Vec<u8>, n: usize, distance: i64, count: usize) -> Result<()> {
    ensure!(
        distance > 0 && distance as usize <= out.len() && count <= n - out.len(),
        "ELG invalid backreference"
    );
    for _ in 0..count {
        out.push(out[out.len() - distance as usize]);
    }
    Ok(())
}
fn stream(c: &mut Cursor<'_>, w: usize, h: usize, rgb: bool) -> Result<Vec<u8>> {
    let unit = if rgb { 3 } else { 1 };
    let n = w * h * unit;
    let mut out = Vec::with_capacity(n);
    loop {
        let flags = c.u8()?;
        if flags == 255 || out.len() == n {
            break;
        }
        let method = flags >> 6;
        match method {
            0 => {
                let count = if flags & 32 != 0 {
                    ((flags as usize & 31) << 8) + c.u8()? as usize + 33
                } else {
                    (flags & 31) as usize + 1
                };
                ensure!(count * unit <= n - out.len(), "ELG literal outside image");
                out.extend_from_slice(c.take(count * unit)?);
            }
            1 => {
                let count = if flags & 32 != 0 {
                    ((flags as usize & 31) << 8) + c.u8()? as usize + if rgb { 34 } else { 35 }
                } else {
                    (flags & 31) as usize + if rgb { 2 } else { 3 }
                };
                ensure!(
                    count * unit <= n - out.len(),
                    "ELG repetition outside image"
                );
                let value = c.take(unit)?;
                for _ in 0..count {
                    out.extend_from_slice(value);
                }
            }
            2 => {
                let sub = (flags >> 4) & 3;
                let (count, distance) = if rgb {
                    match sub {
                        0 => ((flags & 15) as usize + 1, c.u8()? as i64 + 2),
                        1 => {
                            let p = ((flags as i64 & 15) << 8) + c.u8()? as i64 + 2;
                            (c.u8()? as usize + 1, p)
                        }
                        2 => {
                            let a = c.u8()? as i64;
                            let b = c.u8()? as i64;
                            let p = ((((flags as i64 & 15) << 8) + a) << 8) + b + 0x1002;
                            (c.u8()? as usize + 1, p)
                        }
                        _ => {
                            let p = if flags & 8 != 0 {
                                ((flags as i64 & 7) << 8) + c.u8()? as i64 + 10
                            } else {
                                (flags & 7) as i64 + 2
                            };
                            (1, p)
                        }
                    }
                } else {
                    match sub {
                        0 => ((flags & 15) as usize + 2, c.u8()? as i64 + 2),
                        1 => {
                            let p = ((flags as i64 & 15) << 8) + c.u8()? as i64 + 3;
                            (c.u8()? as usize + 4, p)
                        }
                        2 => (3, ((flags as i64 & 15) << 8) + c.u8()? as i64 + 3),
                        _ => (4, ((flags as i64 & 15) << 8) + c.u8()? as i64 + 3),
                    }
                };
                copy(&mut out, n, distance * unit as i64, count * unit)?;
            }
            _ => {
                if !rgb {
                    let (count, p) = if flags & 32 != 0 {
                        (2, (flags & 31) as i64 + 2)
                    } else {
                        (1, (flags & 31) as i64 + 1)
                    };
                    copy(&mut out, n, p, count)?;
                } else {
                    let sub = (flags >> 4) & 3;
                    let (y, x) = if sub == 0 {
                        let sub2 = (flags >> 2) & 3;
                        if sub2 == 3 {
                            let p = ((flags as i64 & 3) << 8) + c.u8()? as i64 + 0x80a;
                            copy(&mut out, n, p * 3, 3)?;
                            continue;
                        }
                        let y = ((flags as i64 & 3) << 8) + c.u8()? as i64 + 16;
                        (
                            y,
                            match sub2 {
                                1 => -1,
                                2 => 1,
                                _ => 0,
                            },
                        )
                    } else {
                        (
                            (flags & 15) as i64 + 1,
                            match sub {
                                2 => -1,
                                3 => 1,
                                _ => 0,
                            },
                        )
                    };
                    copy(&mut out, n, (w as i64 * y - x) * 3, 3)?;
                }
            }
        }
    }
    out.resize(n, 0);
    Ok(out)
}
pub(super) fn decode(b: &[u8]) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 3)?;
    let kind = c.u8()?;
    let (depth, w, h) = match kind {
        2 => {
            let d = c.u8()?;
            let w = c.u16()? as u32;
            let h = c.u16()? as u32;
            c.skip(4)?;
            (d, w, h)
        }
        1 => {
            let d = c.u8()?;
            c.skip(4)?;
            (d, c.u16()? as u32, c.u16()? as u32)
        }
        _ => (kind, c.u16()? as u32, c.u16()? as u32),
    };
    pixels(w, h)?;
    if kind == 2 {
        while c.u8()? != 0 {
            let size = c.u32()? as usize;
            ensure!(size >= 4, "ELG metadata size underflow");
            c.skip(size - 4)?;
        }
    }
    match depth {
        8 => {
            let bytes = stream(&mut c, 1024, 1, false)?;
            let pal = read_palette(&mut Cursor::new(&bytes), 256, 4, true)?;
            palette(w, h, &stream(&mut c, w as usize, h as usize, false)?, &pal)
        }
        24 => raw(
            w,
            h,
            &stream(&mut c, w as usize, h as usize, true)?,
            Format::Bgr,
        ),
        32 => {
            let mut out = raw(
                w,
                h,
                &stream(&mut c, w as usize, h as usize, true)?,
                Format::Bgr,
            )?;
            let mask = stream(&mut c, w as usize, h as usize, false)?;
            for (p, m) in out.rgba.chunks_exact_mut(4).zip(mask) {
                p[3] = m;
            }
            Ok(out)
        }
        _ => bail!("unsupported ELG bit depth"),
    }
}
