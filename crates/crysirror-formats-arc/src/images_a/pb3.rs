// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from arc_unpacker PB3 image decoder, Copyright (C) 2016 rr-.
use super::common::*;
use anyhow::{ensure, Result};
use crysirror_image::{ImageDecodeContext, ImageSurface};
fn read32(b: &[u8], at: usize) -> Result<usize> {
    Ok(C::at(b, at)?.u32()? as usize)
}
fn slice(b: &[u8], at: usize, n: usize) -> Result<&[u8]> {
    C::at(b, at)?.take(n)
}
fn lz(control: &[u8], data: &[u8], n: usize) -> Result<Vec<u8>> {
    ensure!(n <= crysirror_image::MAX_DECODED_BYTES, "PB3 output limit");
    let mut ctl = Bits::new(control, true);
    let mut c = C::new(data);
    let mut dict = [0u8; 2048];
    let mut pos = 0x7de;
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        if ctl.get(1)? != 0 {
            let t = c.u16()? as usize;
            let mut src = t >> 5;
            for _ in 0..((t & 31) + 3).min(n - out.len()) {
                let v = dict[src];
                src = (src + 1) & 2047;
                out.push(v);
                dict[pos] = v;
                pos = (pos + 1) & 2047;
            }
        } else {
            let v = c.u8()?;
            out.push(v);
            dict[pos] = v;
            pos = (pos + 1) & 2047;
        }
    }
    Ok(out)
}
pub fn decode(input: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    ensure!(
        input.len() >= 0x34 + 0x2f && input.starts_with(b"PB3B"),
        "PB3 header"
    );
    let mut b = input.to_vec();
    let tail = b.len() - 0x2f;
    for i in 0..0x2c {
        b[8 + i] = (b[8 + i] ^ input[tail + 0x2c + i % 2]).wrapping_sub(input[tail + i]);
    }
    let mut c = C::at(&b, 0x18)?;
    let sub = c.u32()?;
    let mode = c.u16()?;
    let (w, h, depth) = (u32::from(c.u16()?), u32::from(c.u16()?), c.u16()?);
    ensure!(matches!(depth, 24 | 32), "PB3 unsupported depth");
    let ch = depth as usize / 8;
    let mut out = image(w, h)?;
    let (wi, hi) = (w as usize, h as usize);
    match mode {
        1 => {
            ensure!(sub == 0x10, "PB3 unsupported subtype");
            let ma = read32(&b, 0x2c)?;
            let da = read32(&b, 0x30)?;
            let mut m = C::at(&b, ma)?;
            let mut d = C::at(&b, da)?;
            let mut ms = Vec::new();
            let mut ds = Vec::new();
            for _ in 0..ch {
                ms.push(m.u32()? as usize);
                ds.push(d.u32()? as usize);
            }
            let (mut ma, mut da) = (ma + 4 * ch, da + 4 * ch);
            for channel in 0..ch {
                let main = slice(&b, ma, ms[channel])?;
                let data = slice(&b, da, ds[channel])?;
                ma += ms[channel];
                da += ds[channel];
                let mut c = C::new(main);
                let (a, z, n) = (c.u32()? as usize, c.u32()? as usize, c.u32()? as usize);
                let controls = c.take(a)?;
                let fills = c.take(z)?;
                let plane = lz(c.take(c.left())?, data, n)?;
                let mut ctl = Bits::new(controls, true);
                let mut fill = C::new(fills);
                let mut plane = C::new(&plane);
                for by in (0..hi).step_by(16) {
                    for bx in (0..wi).step_by(16) {
                        let fill = if ctl.get(1)? != 0 {
                            Some(fill.u8()?)
                        } else {
                            None
                        };
                        for y in by..(by + 16).min(hi) {
                            for x in bx..(bx + 16).min(wi) {
                                out.rgba[(y * wi + x) * 4 + [2, 1, 0, 3][channel]] = match fill {
                                    Some(v) => v,
                                    None => plane.u8()?,
                                };
                            }
                        }
                    }
                }
            }
            if ch == 3 {
                for p in out.rgba.chunks_exact_mut(4) {
                    p[3] = 255;
                }
            }
        }
        5 => {
            let mut c = C::at(&b, 0x34)?;
            let mut controls = Vec::new();
            let mut datas = Vec::new();
            for _ in 0..ch {
                controls.push(0x54 + c.u32()? as usize);
                datas.push(0x54 + c.u32()? as usize);
            }
            for channel in 0..ch {
                let ca = controls[channel];
                let da = datas[channel];
                let ce = *controls.get(channel + 1).unwrap_or(&b.len());
                let de = *datas.get(channel + 1).unwrap_or(&b.len());
                ensure!(ce >= ca && de >= da, "PB3 nonmonotonic plane offsets");
                let plane = lz(slice(&b, ca, ce - ca)?, slice(&b, da, de - da)?, wi * hi)?;
                let mut acc = 0u8;
                for (p, v) in out.rgba.chunks_exact_mut(4).zip(plane) {
                    acc = acc.wrapping_add(v);
                    p[[2, 1, 0, 3][channel]] = acc;
                }
            }
        }
        6 => {
            let key = [
                0xa6, 0x75, 0xf3, 0x9c, 0xc5, 0x69, 0x78, 0xa3, 0x3e, 0xa5, 0x4f, 0x79, 0x59, 0xfe,
                0x3a, 0xc7,
            ];
            let name: Vec<u8> = slice(&b, 0x34, 32)?
                .iter()
                .enumerate()
                .map(|(i, v)| v ^ key[i % 16])
                .take_while(|v| *v != 0)
                .collect();
            let stem = match std::str::from_utf8(&name) {
                Ok(s) => s.to_string(),
                Err(_) => {
                    let (s, _, errors) = encoding_rs::SHIFT_JIS.decode(&name);
                    ensure!(!errors, "PB3 reference name encoding");
                    s.into_owned()
                }
            };
            if !stem.is_empty() {
                for ext in ["pb3", "png"] {
                    let path = format!("{stem}.{ext}");
                    if let Some(data) = ctx.try_read_relative(&path)? {
                        let base = ctx.decode_embedded(&path, &data)?;
                        for y in 0..hi.min(base.height as usize) {
                            for x in 0..wi.min(base.width as usize) {
                                let i = (y * wi + x) * 4;
                                let j = (y * base.width as usize + x) * 4;
                                out.rgba[i..i + 4].copy_from_slice(&base.rgba[j..j + 4]);
                            }
                        }
                    }
                }
            }
            let ca = 0x20 + read32(&b, 0xc)?;
            let cs = read32(&b, 0x2c)?;
            let ds = read32(&b, 0x30)?;
            let proxy = lz(slice(&b, ca, cs)?, slice(&b, ca + cs, ds)?, sub as usize)?;
            let mut c = C::new(&proxy);
            let (cs, ds) = (c.u32()? as usize, c.u32()? as usize);
            let mut ctl = Bits::new(c.take(cs)?, true);
            let mut data = C::new(c.take(ds)?);
            for by in (0..hi).step_by(8) {
                for bx in (0..wi).step_by(8) {
                    if ctl.get(1)? == 0 {
                        for y in by..(by + 8).min(hi) {
                            for x in bx..(bx + 8).min(wi) {
                                let p = data.take(4)?;
                                out.rgba[(y * wi + x) * 4..][..4]
                                    .copy_from_slice(&[p[2], p[1], p[0], p[3]]);
                            }
                        }
                    }
                }
            }
        }
        2 | 3 => {
            let (data, mask) = if mode == 2 {
                let ma = read32(&b, 0x2c)?;
                let ms = read32(&b, 0x30)?;
                ensure!(ma >= 0x34, "PB3 JBP1 offset");
                (slice(&b, 0x34, ma - 0x34)?, Some(slice(&b, ma, ms)?))
            } else {
                (&b[0x34..], None)
            };
            let raw = super::jbp1::decode(data)?;
            out = bgr(w, h, &raw, 3, false)?;
            if let Some(mask) = mask {
                let mut m = C::new(mask);
                let mut decoded = Vec::new();
                while m.left() != 0 {
                    let b = m.u8()?;
                    let n = if b == 0 || b == 255 {
                        m.u8()? as usize
                    } else {
                        1
                    };
                    ensure!(decoded.len() + n <= wi * hi, "PB3 mask output overflow");
                    decoded.resize(decoded.len() + n, b);
                }
                ensure!(decoded.len() == wi * hi, "PB3 short mask");
                for (p, a) in out.rgba.chunks_exact_mut(4).zip(decoded) {
                    p[3] = a;
                }
            }
        }
        _ => anyhow::bail!("unsupported PB3 main/subtype {mode}/{sub}"),
    };
    Ok(out)
}
