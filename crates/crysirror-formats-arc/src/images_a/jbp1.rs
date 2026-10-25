// SPDX-License-Identifier: GPL-3.0-or-later
// Safe translation of arc_unpacker JBP1, Copyright (C) 2016 rr-.
use super::common::*;
use anyhow::{ensure, Result};
struct Bit<'a>(Bits<'a>);
impl<'a> Bit<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self(Bits::new(b, false))
    }
    fn get(&mut self, n: usize) -> Result<u32> {
        ensure!(n <= 16, "JBP1 bit count");
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | self.0.get(1)?;
        }
        Ok(v)
    }
}
struct Tree {
    nodes: [[usize; 2]; 31],
    root: usize,
}
impl Tree {
    fn new(mut freq: Vec<u32>) -> Result<Self> {
        let mut nodes = [[0; 2]; 31];
        for size in 16..31 {
            let mut choices: Vec<usize> = (0..size).filter(|&i| freq[i] < 2099999999).collect();
            choices.sort_by_key(|&i| (freq[i], i));
            if choices.len() < 2 {
                return Ok(Self {
                    nodes,
                    root: size - 1,
                });
            }
            let (a, b) = (choices[0], choices[1]);
            nodes[size] = [a, b];
            freq.push(
                freq[a]
                    .checked_add(freq[b])
                    .ok_or_else(|| anyhow::anyhow!("JBP1 frequency overflow"))?,
            );
            freq[a] = 2100000000;
            freq[b] = 2100000000;
        }
        Ok(Self { nodes, root: 30 })
    }
    fn read(&self, b: &mut Bit<'_>) -> Result<usize> {
        let mut at = self.root;
        for _ in 0..32 {
            if at < 16 {
                return Ok(at);
            }
            at = self.nodes[at][b.get(1)? as usize];
        }
        anyhow::bail!("JBP1 tree cycle")
    }
}
fn idct(v: [i64; 8], shift: u32) -> [i16; 8] {
    let a = v[0];
    let b = v[4];
    let mut c = v[2];
    let mut d = v[6];
    let mut x = ((c + d) * 35467) >> 16;
    c = ((c * 50159) >> 16) + x;
    d = ((d * -121094) >> 16) + x;
    let w = a + b + c;
    x = a + b - c;
    let y = a - b + d;
    let z = a - b - d;
    let (d, b, c, a) = (v[5], v[1], v[7], v[3]);
    let n = ((a + b + c + d) * 77062) >> 16;
    let s = n + ((a * 201373) >> 16) + (((a + c) * -128553) >> 16) + (((a + d) * -167963) >> 16);
    let t = n + ((b * 98390) >> 16) + (((b + d) * -25570) >> 16) + (((b + c) * -58980) >> 16);
    let u = n + ((c * 19571) >> 16) + (((b + c) * -58980) >> 16) + (((a + c) * -128553) >> 16);
    let v = n + ((d * 134553) >> 16) + (((b + d) * -25570) >> 16) + (((a + d) * -167963) >> 16);
    [
        (w + t) >> shift,
        (y + s) >> shift,
        (z + v) >> shift,
        (x + u) >> shift,
        (x - u) >> shift,
        (z - v) >> shift,
        (y - s) >> shift,
        (w - t) >> shift,
    ]
    .map(|v| v as i16)
}
fn dct(d: &mut [i16; 64], q: &[i16; 64]) {
    for x in 0..8 {
        let mut v = [0; 8];
        for y in 0..8 {
            v[y] = i64::from(d[y * 8 + x]) * i64::from(q[y * 8 + x]);
        }
        let v = idct(v, 0);
        for y in 0..8 {
            d[y * 8 + x] = v[y];
        }
    }
    for row in d.chunks_exact_mut(8) {
        let v = idct(std::array::from_fn(|i| i64::from(row[i])), 3);
        row.copy_from_slice(&v);
    }
}
fn signed(bits: usize, b: &mut Bit<'_>) -> Result<i32> {
    if bits == 0 {
        return Ok(0);
    }
    let n = b.get(bits)? as i32;
    Ok(if n < (1 << (bits - 1)) {
        n - (1 << bits) + 1
    } else {
        n
    })
}
pub fn decode(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut c = C::new(bytes);
    ensure!(c.take(4)? == b"JBP1", "JBP1 signature");
    let at = c.u32()? as usize;
    let flags = c.u32()?;
    c.skip(4)?;
    let (w, h, depth) = (u32::from(c.u16()?), u32::from(c.u16()?), c.u16()?);
    let _ = image(w, h)?;
    ensure!(depth == 24, "unsupported JBP1 depth");
    c.skip(6)?;
    let (n1, n2) = (c.u32()? as usize, c.u32()? as usize);
    let (xpad, ypad) = match (flags >> 28) & 3 {
        0 => (8, 8),
        1 => (16, 16),
        2 => (32, 16),
        _ => anyhow::bail!("unsupported JBP1 block layout"),
    };
    let (w, h) = (w as usize, h as usize);
    let bw = w.div_ceil(xpad) * xpad;
    let bh = h.div_ceil(ypad) * ypad;
    ensure!(
        bw * bh * 4 <= crysirror_image::MAX_DECODED_BYTES,
        "JBP1 padded output limit"
    );
    let (xb, yb) = (bw / 16, bh / 16);
    let mut c = C::at(bytes, at)?;
    let mut fd = Vec::new();
    let mut fa = Vec::new();
    for _ in 0..16 {
        fd.push(c.u32()?);
    }
    for _ in 0..16 {
        fa.push(c.u32()?);
    }
    let tree_input: Vec<u8> = c.take(16)?.iter().map(|v| v.wrapping_add(1)).collect();
    ensure!(
        flags & 0x08000000 != 0,
        "JBP1 absent quantization table is undefined in pinned decoder"
    );
    let mut qy = [0i16; 64];
    let mut qc = qy;
    for v in &mut qy {
        *v = i16::from(c.u8()?);
    }
    for v in &mut qc {
        *v = i16::from(c.u8()?);
    }
    let (mut b1, mut b2) = (Bit::new(c.take(n1)?), Bit::new(c.take(n2)?));
    let (td, ta) = (Tree::new(fd)?, Tree::new(fa)?);
    let mut dc = Vec::new();
    let mut acc = 0i32;
    for _ in 0..xb * yb * 6 {
        acc = acc.wrapping_add(signed(td.read(&mut b1)?, &mut b1)?);
        dc.push(acc as i16);
    }
    let order = [
        1usize, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34,
        27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44,
        51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63, 0,
    ];
    let mut blocks = vec![0u8; bw * bh * 4];
    for by in 0..yb {
        for bx in 0..xb {
            let mut tables = [[0i16; 64]; 6];
            for n in 0..6 {
                tables[n][0] = dc[(by * xb + bx) * 6 + n];
                let mut i = 0;
                while i < 63 {
                    let bits = ta.read(&mut b2)?;
                    if bits == 15 {
                        break;
                    }
                    if bits == 0 {
                        let mut at = 0;
                        while b2.get(1)? != 0 {
                            at += 1;
                            ensure!(at < 16, "JBP1 run index");
                        }
                        i += tree_input[at] as usize;
                    } else {
                        tables[n][order[i]] = signed(bits, &mut b2)? as i16;
                        i += 1;
                    }
                }
                dct(&mut tables[n], if n < 4 { &qy } else { &qc });
            }
            for y in 0..16 {
                for x in 0..16 {
                    let yy = i64::from(tables[(y / 8) * 2 + x / 8][(y % 8) * 8 + x % 8]) + 128;
                    let ci = (y / 2) * 8 + x / 2;
                    let c = i64::from(tables[5][ci]);
                    let d = i64::from(tables[4][ci]);
                    let r = (c * 0x166f0) >> 16;
                    let g = ((d * 0x5810) >> 16) + ((c * 0xb6c0) >> 16);
                    let b = (d * 0x1c590) >> 16;
                    ensure!(
                        [yy + b, yy - g, yy + r]
                            .into_iter()
                            .all(|v| (-256..=511).contains(&v)),
                        "JBP1 color lookup out of range"
                    );
                    let i = ((by * 16 + y) * bw + bx * 16 + x) * 4;
                    blocks[i..i + 3].copy_from_slice(&[
                        (yy + b).clamp(0, 255) as u8,
                        (yy - g).clamp(0, 255) as u8,
                        (yy + r).clamp(0, 255) as u8,
                    ]);
                }
            }
        }
    }
    let mut raw = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            raw.extend_from_slice(&blocks[(y * bw + x) * 4..][..3]);
        }
    }
    Ok(raw)
}
