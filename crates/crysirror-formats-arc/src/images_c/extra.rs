// SPDX-License-Identifier: GPL-3.0-or-later
// Adapted from arc_unpacker at the documented pin, Copyright (C) 2016 rr-.
use super::common::*;
use anyhow::{bail, ensure, Result};
use crysirror_image::ImageSurface;
fn kg_int(b: &mut Bits<'_>) -> Result<usize> {
    let v = b.get(2)?;
    if v != 0 {
        return Ok(v);
    }
    let v = b.get(4)?;
    if v != 0 {
        return Ok(v + 3);
    }
    let v = b.get(8)?;
    if v != 0 {
        return Ok(v);
    }
    let v = b.get(16)?;
    if v != 0 {
        return Ok(v);
    }
    Ok((b.get(16)? << 16) | b.get(16)?)
}
fn kg_channel(b: &mut Bits<'_>, table: &mut [[u8; 8]], w: u32, h: u32) -> Result<Vec<u8>> {
    let n = pixels(w, h)?;
    ensure!(n >= 2, "KG needs at least two pixels");
    let mut out = vec![b.get(8)? as u8, b.get(8)? as u8];
    while out.len() < n {
        let mode = if b.get(1)? == 0 {
            0
        } else if b.get(1)? == 0 {
            2
        } else {
            12 + b.get(2)?
        };
        if mode == 0 {
            let row = &mut table[*out.last().unwrap() as usize];
            let v = if b.get(1)? != 0 {
                b.get(8)? as u8
            } else {
                row[b.get(3)?]
            };
            let pos = row.iter().position(|&x| x == v).unwrap_or(7);
            row.copy_within(0..pos, 1);
            row[0] = v;
            out.push(v);
        } else {
            let count = kg_int(b)?;
            let distance = match mode {
                2 => 1,
                12 => w as i64,
                13 => w as i64 - 1,
                14 => w as i64 + 1,
                15 => 2,
                _ => unreachable!(),
            };
            ensure!(
                count > 0
                    && count <= n - out.len()
                    && distance > 0
                    && distance as usize <= out.len(),
                "invalid KG run"
            );
            for _ in 0..count {
                out.push(out[out.len() - distance as usize]);
            }
        }
    }
    Ok(out)
}
pub(super) fn kg(b: &[u8]) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 3)?;
    let kind = c.u8()?;
    let w = c.u16()? as u32;
    let h = c.u16()? as u32;
    let n = pixels(w, h)?;
    c.skip(4)?;
    let palpos = c.u32()? as usize;
    let datapos = c.u32()? as usize;
    c.skip(24)?;
    let alpha = c.u32()? as usize;
    let mut table = vec![[0, 1, 2, 3, 4, 5, 6, 7]; 256];
    let mut bits = Bits::new(Cursor::at(b, datapos)?.rest());
    let mut out = match kind {
        1 => {
            let indices = kg_channel(&mut bits, &mut table, w, h)?;
            let pal = read_palette(&mut Cursor::at(b, palpos)?, 256, 4, true)?;
            palette(w, h, &indices, &pal)?
        }
        2 => {
            let channels = [
                kg_channel(&mut bits, &mut table, w, h)?,
                kg_channel(&mut bits, &mut table, w, h)?,
                kg_channel(&mut bits, &mut table, w, h)?,
            ];
            let mut data = Vec::with_capacity(n * 3);
            for i in 0..n {
                for ch in &channels {
                    data.push(ch[i]);
                }
            }
            raw(w, h, &data, Format::Bgr)?
        }
        _ => bail!("unsupported KG type"),
    };
    if alpha != 0 {
        let mut table = vec![[0, 1, 2, 3, 4, 5, 6, 7]; 256];
        let mask = kg_channel(
            &mut Bits::new(Cursor::at(b, alpha)?.rest()),
            &mut table,
            w,
            h,
        )?;
        for (p, m) in out.rgba.chunks_exact_mut(4).zip(mask) {
            p[3] = m;
        }
    }
    flip(&mut out);
    Ok(out)
}
pub(super) fn bsg(b: &[u8]) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 0x16)?;
    let w = c.u16()? as u32;
    let h = c.u16()? as u32;
    let n = pixels(w, h)?;
    c.skip(22)?;
    let color = c.u8()?;
    let compression = c.u8()?;
    ensure!(compression <= 1, "unknown BSG compression");
    let pos = c.u32()? as usize;
    let size = c.u32()? as usize;
    let palpos = c.u32()? as usize;
    let channels = match color {
        0 => 4,
        1 => 3,
        2 => 1,
        _ => bail!("unknown BSG color type"),
    };
    let mut c = Cursor::at(b, pos)?;
    let mut data = vec![0; n * channels];
    for ch in 0..channels {
        let values = if compression == 0 {
            ensure!(
                size % channels == 0 && size / channels == n,
                "BSG channel size mismatch"
            );
            c.take(n)?.to_vec()
        } else {
            let stored = bounded(c.u32()? as usize)?;
            let mut r = Cursor::new(c.take(stored)?);
            let mut values = Vec::with_capacity(n);
            while !r.rest().is_empty() {
                let count = r.u8()? as i8;
                if count >= 0 {
                    values.extend_from_slice(r.take(count as usize + 1)?);
                } else {
                    let value = r.u8()?;
                    values.resize(values.len() + (1isize - count as isize) as usize, value);
                }
                ensure!(values.len() <= n, "BSG RLE exceeds channel");
            }
            ensure!(values.len() == n, "truncated BSG channel");
            values
        };
        for (i, v) in values.into_iter().enumerate() {
            data[i * channels + ch] = v;
        }
    }
    let mut out = if channels == 1 && palpos != 0 {
        let pal = read_palette(&mut Cursor::at(b, palpos)?, 256, 4, false)?;
        palette(w, h, &data, &pal)?
    } else {
        raw(
            w,
            h,
            &data,
            match channels {
                1 => Format::Gray,
                3 => Format::Bgr,
                _ => Format::Bgra,
            },
        )?
    };
    flip(&mut out);
    Ok(out)
}
struct CronusHeader {
    w: u32,
    h: u32,
    depth: u32,
    size: usize,
    pos: usize,
    enc: u8,
    alpha: bool,
}
fn cronus_header(b: &[u8]) -> Result<CronusHeader> {
    for (keys, enc) in [
        ([0xa53cc35a, 0x35421005, 0xcf42355d], 1),
        ([0x2468fcda, 0x4fc2cc4d, 0xcf42355d], 2),
        ([0, 0, 0], 0),
    ] {
        let candidate = (|| -> Result<CronusHeader> {
            let mut c = Cursor::new(b);
            let (w, h, depth, size, alpha) = if enc != 0 {
                let w = c.u32()? ^ keys[0];
                let h = c.u32()? ^ keys[1];
                let d = c.u32()?;
                c.skip(4)?;
                let size = c.u32()? ^ keys[2];
                c.skip(4)?;
                (w, h, d, size, false)
            } else {
                c.skip(4)?;
                let size = c.u32()?;
                c.skip(8)?;
                let w = c.u32()?;
                let h = c.u32()?;
                let d = c.u32()?;
                c.skip(8)?;
                let alpha = c.u32()? != 0;
                (w, h, d, size, alpha)
            };
            let n = pixels(w, h)?;
            let expected = match depth {
                8 => n + 1024,
                24 => n * 3,
                32 => n * 4,
                _ => bail!("Cronus depth"),
            };
            ensure!(size as usize == expected, "Cronus decoded size mismatch");
            Ok(CronusHeader {
                w,
                h,
                depth,
                size: expected,
                pos: c.pos,
                enc,
                alpha,
            })
        })();
        if let Ok(h) = candidate {
            return Ok(h);
        }
    }
    bail!("no recognized Cronus header")
}
pub(super) fn cronus_probe(b: &[u8]) -> bool {
    cronus_header(b).is_ok()
}
pub(super) fn cronus(b: &[u8], path: &str) -> Result<ImageSurface> {
    let h = cronus_header(b)?;
    let mut data = b[h.pos..].to_vec();
    if h.enc == 1 {
        let end = h.size.min(data.len()) / 2 * 2;
        for word in data[..end].chunks_exact_mut(2) {
            let v = u16::from_le_bytes([word[0], word[1]]).swap_bytes() ^ 0x33cc;
            word.copy_from_slice(&v.to_le_bytes());
        }
    } else if h.enc == 2 {
        let name = path.rsplit('/').next().unwrap_or(path);
        ensure!(
            name.is_ascii(),
            "Cronus filename-byte key is only verified for ASCII names"
        );
        let bytes = name.as_bytes();
        let mut key = bytes.iter().map(|&b| b as u32).sum::<u32>();
        let delta = key % 32;
        for byte in &mut data {
            *byte ^= key as u8;
            key = key.wrapping_add(delta);
        }
    }
    let data = lzss(&data, h.size, false)?;
    let mut out = if h.depth == 8 {
        let mut c = Cursor::new(&data);
        let pal = read_palette(&mut c, 256, 4, false)?;
        palette(h.w, h.h, c.rest(), &pal)?
    } else {
        raw(
            h.w,
            h.h,
            &data,
            if h.depth == 24 {
                Format::Bgr
            } else {
                Format::Bgra
            },
        )?
    };
    if !h.alpha {
        for p in out.rgba.chunks_exact_mut(4) {
            p[3] = 255;
        }
    }
    if h.enc != 0 {
        flip(&mut out);
    }
    Ok(out)
}
fn gd_custom(bits: &mut Bits<'_>, w: usize, h: usize) -> Result<Vec<u8>> {
    let n = w * h * 3;
    let mut data = vec![255; n];
    let mut pos = 0usize;
    loop {
        let mut skip = bits.get(2)?;
        if skip == 2 {
            skip = bits.get(2)? + 2;
        } else if skip == 3 {
            let mut width = 3;
            while bits.get(1)? != 0 {
                width += 1;
                if width >= 24 {
                    break;
                }
            }
            if width >= 24 {
                break;
            }
            skip = (1 << width | bits.get(width)?) - 2;
        }
        pos = pos
            .checked_add(skip * 3)
            .ok_or_else(|| anyhow::anyhow!("GD skip overflow"))?;
        ensure!(pos + 3 <= n, "GD color outside canvas");
        let value = [bits.get(8)? as u8, bits.get(8)? as u8, bits.get(8)? as u8];
        data[pos..pos + 3].copy_from_slice(&value);
        if bits.get(1)? != 0 {
            let mut target = pos;
            loop {
                let mode = bits.get(2)?;
                let step = match mode {
                    0 => {
                        if bits.get(1)? == 0 {
                            break;
                        }
                        if bits.get(1)? != 0 {
                            w + 2
                        } else {
                            w - 2
                        }
                    }
                    1 => w - 1,
                    2 => w,
                    _ => w + 1,
                };
                target = target
                    .checked_add(step * 3)
                    .ok_or_else(|| anyhow::anyhow!("GD chain overflow"))?;
                ensure!(target + 3 <= n, "GD chain outside canvas");
                data[target..target + 3].copy_from_slice(&value);
            }
        }
        pos += 3;
    }
    let mut previous = [0; 3];
    for p in data.chunks_exact_mut(3) {
        if p != [255; 3] {
            previous.copy_from_slice(p);
        } else {
            p.copy_from_slice(&previous);
        }
    }
    Ok(data)
}
pub(super) fn gd(b: &[u8]) -> Result<ImageSurface> {
    let (w, h) = if b.get(2) == Some(&b'2') {
        (640, 480)
    } else {
        (800, 600)
    };
    let n = pixels(w, h)? * 3;
    let mut c = Cursor::at(b, 4 + 3 * (w as usize / 10) * (h as usize / 10 - 1))?;
    let kind = c.u8()?;
    c.skip(1)?;
    let data = match kind {
        0x62 => c.take(n)?.to_vec(),
        0x70 => gd_custom(&mut Bits::new(c.rest()), w as usize, h as usize)?,
        0x6c => {
            let mut bits = Bits::new(c.rest());
            let mut dict = vec![0; 65536];
            let mut write = 1;
            let mut out = Vec::with_capacity(n);
            while out.len() < n {
                if bits.get(1)? != 0 {
                    let v = bits.get(8)? as u8;
                    out.push(v);
                    dict[write] = v;
                    write = (write + 1) & 65535;
                } else {
                    let mut read = bits.get(16)?;
                    let count = bits.get(4)? + 3;
                    for _ in 0..count.min(n - out.len()) {
                        let v = dict[read];
                        read = (read + 1) & 65535;
                        out.push(v);
                        dict[write] = v;
                        write = (write + 1) & 65535;
                    }
                }
            }
            out
        }
        _ => bail!("unknown GD compression"),
    };
    let mut out = raw(w, h, &data, Format::Bgr)?;
    flip(&mut out);
    Ok(out)
}
