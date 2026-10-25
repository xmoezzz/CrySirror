// SPDX-License-Identifier: GPL-3.0-or-later
// Safe CPU translation of arc_unpacker TLG5/6 decoders, Copyright (C) 2016 rr-.
use super::common::*;
use anyhow::{ensure, Context, Result};
use crysirror_image::{ImageDecodeContext, ImageSurface};
struct Lz {
    dict: [u8; 4096],
    pos: usize,
}
impl Lz {
    fn new() -> Self {
        Self {
            dict: [0; 4096],
            pos: 0,
        }
    }
    fn decode(&mut self, b: &[u8], n: usize) -> Result<Vec<u8>> {
        let mut c = C::new(b);
        let mut o = Vec::with_capacity(n);
        while o.len() < n {
            let f = c.u8()?;
            for bit in 0..8 {
                if o.len() == n {
                    break;
                }
                if f & (1 << bit) != 0 {
                    let a = c.u8()? as usize;
                    let z = c.u8()? as usize;
                    let mut at = a | ((z & 15) << 8);
                    let mut len = 3 + (z >> 4);
                    if len == 18 {
                        len += c.u8()? as usize;
                    }
                    for _ in 0..len.min(n - o.len()) {
                        let v = self.dict[at];
                        at = (at + 1) & 4095;
                        o.push(v);
                        self.dict[self.pos] = v;
                        self.pos = (self.pos + 1) & 4095;
                    }
                } else {
                    let v = c.u8()?;
                    o.push(v);
                    self.dict[self.pos] = v;
                    self.pos = (self.pos + 1) & 4095;
                }
            }
        }
        Ok(o)
    }
}
pub fn decode(bytes: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    let mut c = C::new(bytes);
    let sig = c.take(11)?;
    match sig {
        b"TLG0.0\0sds\x1a" => {
            let n = c.u32()? as usize;
            let raw = c.take(n)?;
            while c.left() != 0 {
                ensure!(c.take(4)? == b"tags", "unknown TLG SDS chunk");
                let n = c.u32()? as usize;
                c.skip(n)?;
            }
            ctx.decode_embedded("payload.tlg", raw)
        }
        b"TLG5.0\0raw\x1a" => v5(&mut c),
        b"TLG6.0\0raw\x1a" => v6(&mut c),
        _ => anyhow::bail!("unsupported TLG magic"),
    }
}
fn v5(c: &mut C<'_>) -> Result<ImageSurface> {
    let channels = c.u8()? as usize;
    ensure!(matches!(channels, 3 | 4), "TLG5 channels");
    let (w, h, bh) = (c.u32()?, c.u32()?, c.u32()?);
    let mut out = image(w, h)?;
    ensure!(bh > 0, "TLG5 zero block height");
    let (w, h, bh) = (w as usize, h as usize, bh as usize);
    ensure!(
        w.checked_mul(bh)
            .is_some_and(|n| n <= crysirror_image::MAX_DECODED_BYTES),
        "TLG5 block limit"
    );
    c.skip(h.div_ceil(bh) * 4)?;
    let mut lz = Lz::new();
    for y in (0..h).step_by(bh) {
        let rows = bh.min(h - y);
        let mut planes = Vec::new();
        for _ in 0..channels {
            let flag = c.u8()?;
            let n = c.u32()? as usize;
            let packed = c.take(n)?;
            let p = if flag == 0 {
                lz.decode(packed, w * rows)?
            } else {
                packed.to_vec()
            };
            ensure!(p.len() >= w * rows, "TLG5 short plane");
            planes.push(p);
        }
        for yy in 0..rows {
            let mut prev = [0u8; 4];
            for x in 0..w {
                let at = yy * w + x;
                let g = planes[1][at];
                let res = [
                    planes[0][at].wrapping_add(g),
                    g,
                    planes[2][at].wrapping_add(g),
                    if channels == 4 { planes[3][at] } else { 0 },
                ];
                for k in 0..channels {
                    prev[k] = prev[k].wrapping_add(res[k]);
                }
                let i = ((y + yy) * w + x) * 4;
                for (k, ch) in [2, 1, 0, 3].into_iter().enumerate() {
                    out.rgba[i + ch] = if k == 3 && channels == 3 {
                        255
                    } else {
                        prev[k].wrapping_add(if y + yy == 0 {
                            0
                        } else {
                            out.rgba[i - w * 4 + ch]
                        })
                    };
                }
            }
        }
    }
    Ok(out)
}
const TABLE: [[usize; 9]; 4] = [
    [3, 7, 15, 27, 63, 108, 223, 448, 130],
    [3, 5, 13, 24, 51, 95, 192, 384, 257],
    [2, 5, 12, 21, 39, 86, 155, 320, 384],
    [2, 3, 9, 18, 33, 61, 129, 258, 511],
];
fn unary(b: &mut Bits<'_>) -> Result<usize> {
    let mut z = 0;
    while b.get(1)? == 0 {
        z += 1;
        ensure!(z < 32, "TLG Golomb run overflow at bit {}", b.p);
    }
    Ok(z)
}
fn golomb(bytes: &[u8], pixels: &mut [[u8; 4]], ch: usize) -> Result<()> {
    let mut b = Bits::new(bytes, false);
    let mut zero = b.get(1)? == 0;
    let (mut at, mut a, mut n) = (0, 0usize, 3usize);
    while at < pixels.len() {
        let z = unary(&mut b).context("TLG run length")?;
        let count = ((1usize << z) + b.get(z)? as usize).min(pixels.len() - at);
        for _ in 0..count {
            if !zero {
                if a >= 1024 {
                    a = 0;
                }
                let mut sum = 0;
                let k = TABLE[n]
                    .iter()
                    .position(|&v| {
                        sum += v;
                        a < sum
                    })
                    .context("TLG Golomb state")?;
                let base = b.p / 8;
                let shift = b.p % 8;
                let mut peek = 0u32;
                for j in 0..4 {
                    peek |= u32::from(*bytes.get(base + j).unwrap_or(&0)) << (j * 8);
                }
                let z = if peek >> shift == 0 {
                    let z = usize::from(*bytes.get(base + 4).context("truncated TLG escape")?);
                    b.p = (base + 5) * 8;
                    z
                } else {
                    unary(&mut b).context("TLG nonzero value")?
                };
                let raw = (z << k) + b.get(k)? as usize;
                let sign = (raw as i32 & 1) - 1;
                let value = (raw >> 1) as i32;
                a += raw >> 1;
                pixels[at][ch] = ((value ^ sign) + sign + 1) as u8;
                if n == 0 {
                    a >>= 1;
                    n = 3;
                } else {
                    n -= 1;
                }
            }
            at += 1;
        }
        zero = !zero;
    }
    Ok(())
}
fn transform(p: &mut [u8; 4], t: u8) -> Result<()> {
    let [mut b, mut g, mut r, a] = *p;
    match t {
        0 => (),
        1 => {
            r = r.wrapping_add(g);
            b = b.wrapping_add(g)
        }
        2 => {
            g = g.wrapping_add(b);
            r = r.wrapping_add(g)
        }
        3 => {
            g = g.wrapping_add(r);
            b = b.wrapping_add(g)
        }
        4 => {
            b = b.wrapping_add(r);
            g = g.wrapping_add(b);
            r = r.wrapping_add(g)
        }
        5 => {
            b = b.wrapping_add(r);
            g = g.wrapping_add(b)
        }
        6 => b = b.wrapping_add(g),
        7 => g = g.wrapping_add(b),
        8 => r = r.wrapping_add(g),
        9 => {
            r = r.wrapping_add(b);
            g = g.wrapping_add(r);
            b = b.wrapping_add(g)
        }
        10 => {
            b = b.wrapping_add(r);
            g = g.wrapping_add(r)
        }
        11 => {
            r = r.wrapping_add(b);
            g = g.wrapping_add(b)
        }
        12 => {
            r = r.wrapping_add(b);
            g = g.wrapping_add(r)
        }
        13 => {
            b = b.wrapping_add(g);
            r = r.wrapping_add(b);
            g = g.wrapping_add(r)
        }
        14 => {
            g = g.wrapping_add(r);
            b = b.wrapping_add(g);
            r = r.wrapping_add(b)
        }
        15 => {
            g = g.wrapping_add(b.wrapping_mul(2));
            r = r.wrapping_add(b.wrapping_mul(2))
        }
        _ => anyhow::bail!("unsupported TLG filter"),
    };
    *p = [b, g, r, a];
    Ok(())
}
fn v6(c: &mut C<'_>) -> Result<ImageSurface> {
    let channels = c.u8()? as usize;
    ensure!(matches!(channels, 3 | 4), "TLG6 channels");
    ensure!(
        c.take(3)? == [0, 0, 0],
        "unsupported TLG6 flags/color/table"
    );
    let (w, h) = (c.u32()?, c.u32()?);
    let _maxbits = c.u32()?;
    let mut out = image(w, h)?;
    let (w, h) = (w as usize, h as usize);
    let (xb, yb) = (w.div_ceil(8), h.div_ceil(8));
    let n = c.u32()? as usize;
    let mut lz = Lz::new();
    let mut at = 0;
    for i in 0..32 {
        for j in 0..16 {
            for v in [i, i, i, i, j, j, j, j] {
                lz.dict[at] = v;
                at += 1;
            }
        }
    }
    let filters = lz.decode(c.take(n)?, xb * yb)?;
    let mut output = vec![[0u8; 4]; w * h];
    for y in (0..h).step_by(8) {
        let bh = 8.min(h - y);
        let mut residual = vec![[0u8; 4]; w * bh];
        for ch in 0..channels {
            let bits = c.u32()?;
            ensure!(bits >> 30 == 0, "unsupported TLG entropy method");
            let n = (bits as usize).div_ceil(8);
            golomb(c.take(n)?, &mut residual, ch)
                .with_context(|| format!("TLG6 y={y} channel={ch} bits={bits}"))?;
        }
        for yy in y..y + bh {
            let odd = (y + bh - yy - 1) as isize - (yy - y) as isize;
            let step = if yy & 1 == 0 { 1isize } else { -1 };
            for (start, end, base) in [(0, w / 8, 8 * (yy - y)), (w / 8, xb, (w % 8) * (yy - y))] {
                if start == end {
                    continue;
                }
                let mut input = base as isize + (bh * 8 * start) as isize;
                let mut x = start * 8;
                let mut left = if start > 0 {
                    output[yy * w + x - 1]
                } else {
                    [0, 0, 0, if channels == 3 { 255 } else { 0 }]
                };
                let mut tl = if start > 0 && yy > 0 {
                    output[(yy - 1) * w + x - 1]
                } else {
                    [0, 0, 0, if channels == 3 { 255 } else { 0 }]
                };
                for block in start..end {
                    let ww = 8.min(w - block * 8);
                    if step < 0 {
                        input += ww as isize - 1;
                    }
                    if block & 1 != 0 {
                        input += odd * ww as isize;
                    }
                    let filter = filters[(y / 8) * xb + block];
                    for _ in 0..ww {
                        let mut r = *residual
                            .get(usize::try_from(input).context("negative TLG residual index")?)
                            .context("TLG residual index")?;
                        transform(&mut r, filter >> 1)?;
                        let top = if yy > 0 {
                            output[(yy - 1) * w + x]
                        } else {
                            [0; 4]
                        };
                        for k in 0..4 {
                            let (a, b, c) =
                                (i32::from(left[k]), i32::from(top[k]), i32::from(tl[k]));
                            let pred = if filter & 1 != 0 {
                                (a + b + 1) / 2
                            } else if c >= a.max(b) {
                                a.min(b)
                            } else if c <= a.min(b) {
                                a.max(b)
                            } else {
                                a + b - c
                            };
                            left[k] = (pred as u8).wrapping_add(r[k]);
                        }
                        if channels == 3 {
                            left[3] = 255;
                        }
                        tl = top;
                        output[yy * w + x] = left;
                        x += 1;
                        input += step;
                    }
                    input += (bh * 8) as isize + if step > 0 { -(ww as isize) } else { 1 };
                    if block & 1 != 0 {
                        input -= odd * ww as isize;
                    }
                }
            }
        }
    }
    for (p, bgra) in out.rgba.chunks_exact_mut(4).zip(output) {
        p.copy_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
    }
    Ok(out)
}
