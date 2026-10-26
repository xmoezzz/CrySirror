// SPDX-License-Identifier: GPL-3.0-or-later
// Translated from arc_unpacker, Copyright (C) 2016 rr-.
use super::{at, u16at, u32at};
use crate::common::Cursor;
use anyhow::{ensure, Context, Result};
pub(crate) struct Bits<'a> {
    pub c: Cursor<'a>,
    byte: u8,
    left: u8,
}
impl<'a> Bits<'a> {
    pub fn new(d: &'a [u8]) -> Self {
        Self {
            c: Cursor::new(d),
            byte: 0,
            left: 0,
        }
    }
    pub fn get(&mut self, n: usize) -> Result<u32> {
        ensure!(n <= 32, "bit width");
        let mut v = 0;
        for _ in 0..n {
            if self.left == 0 {
                self.byte = self.c.u8()?;
                self.left = 8;
            }
            self.left -= 1;
            v = (v << 1) | ((self.byte >> self.left) & 1) as u32;
        }
        Ok(v)
    }
}
fn tree(b: &mut Bits<'_>, nodes: &mut Vec<[usize; 2]>, depth: usize) -> Result<usize> {
    ensure!(depth <= 256, "Huffman depth");
    if b.get(1)? == 0 {
        return Ok(b.get(8)? as usize);
    }
    ensure!(nodes.len() < 256, "Huffman tree size");
    let i = nodes.len();
    nodes.push([0; 2]);
    nodes[i] = [tree(b, nodes, depth + 1)?, tree(b, nodes, depth + 1)?];
    Ok(i + 256)
}
pub fn huffman(d: &[u8], n: usize) -> Result<Vec<u8>> {
    let mut b = Bits::new(d);
    let mut nodes = Vec::new();
    let root = tree(&mut b, &mut nodes, 0)?;
    let mut o = Vec::with_capacity(n);
    while o.len() < n {
        let mut symbol = root;
        while symbol >= 256 {
            symbol = nodes[symbol - 256][b.get(1)? as usize];
        }
        o.push(symbol as u8);
    }
    Ok(o)
}
pub fn lnd(d: &[u8], n: usize) -> Result<Vec<u8>> {
    let mut c = Cursor::new(d);
    let mut o = Vec::with_capacity(n);
    while o.len() < n {
        let b = c.u8()?;
        if b & 0x80 != 0 {
            if b & 0x40 != 0 {
                let mut count = (b & 31) as usize + 2;
                if b & 32 != 0 {
                    count += (c.u8()? as usize) << 5;
                }
                let v = c.u8()?;
                o.resize((o.len() + count).min(n), v);
            } else {
                let count = ((b >> 2) & 15) as usize + 2;
                let dist = (((b & 3) as usize) << 8) + c.u8()? as usize + 1;
                copy(&mut o, dist, count, n)?;
            }
        } else if b & 0x40 != 0 {
            let repeat = c.u8()? as usize + 1;
            let block = c.take((b & 63) as usize + 2)?;
            for _ in 0..repeat {
                let count = block.len().min(n - o.len());
                o.extend_from_slice(&block[..count]);
            }
        } else {
            let mut count = (b & 31) as usize + 1;
            if b & 32 != 0 {
                count += (c.u8()? as usize) << 5;
            }
            o.extend_from_slice(c.take(count.min(n - o.len()))?);
        }
    }
    Ok(o)
}
pub fn copy(o: &mut Vec<u8>, dist: usize, count: usize, n: usize) -> Result<()> {
    ensure!(dist > 0 && dist <= o.len(), "back-reference outside output");
    for _ in 0..count.min(n - o.len()) {
        o.push(o[o.len() - dist]);
    }
    Ok(())
}
pub fn pak2(d: &[u8], n: usize) -> Result<Vec<u8>> {
    let mut c = Cursor::new(d);
    let mut o = Vec::with_capacity(n);
    while o.len() < n {
        let b = c.u8()?;
        if b >= 0xa0 {
            let (count, v) = if b == 255 {
                (c.u8()? as usize + 2, 0)
            } else if b >= 0xe0 {
                ((b & 31) as usize + 2, 0)
            } else {
                ((b & 31) as usize + 2, c.u8()?)
            };
            ensure!(o.len() + count <= n, "PAK2 run exceeds size");
            o.resize(o.len() + count, v);
        } else if b >= 0x80 {
            let count = (b & 31) as usize;
            ensure!(count <= n - o.len(), "PAK2 literal exceeds size");
            o.extend_from_slice(c.take(count)?);
        } else {
            let dist = (c.u8()? as usize + ((b as usize) << 8)) % 0x400;
            copy(&mut o, dist, (b >> 2) as usize + 2, n)?;
        }
    }
    Ok(o)
}
pub fn cps(d: &[u8], limit: usize) -> Result<Vec<u8>> {
    let packed = u32at(d, 4)? as usize;
    ensure!(packed >= 20, "CPS compressed size");
    let compression = u16at(d, 10)?;
    let n = u32at(d, 12)? as usize;
    ensure!(n <= limit, "CPS output limit");
    let mut o = at(d, 16, packed - 20)?.to_vec();
    let offset = u32at(d, packed - 4)?.wrapping_sub(0x7534682) as usize;
    if offset != 0 {
        let real = offset.checked_sub(16).context("CPS key offset")?;
        let mut key = u32at(&o, real)?
            .wrapping_add(offset as u32)
            .wrapping_add(0x3786425);
        ensure!(o.len() % 4 == 0, "CPS word alignment");
        for (i, word) in o.chunks_exact_mut(4).enumerate() {
            let mut v = u32::from_le_bytes(word.try_into().unwrap());
            if i * 4 != real {
                v = v.wrapping_sub(packed as u32).wrapping_sub(key);
            }
            word.copy_from_slice(&v.to_le_bytes());
            key = key.wrapping_mul(0x41c64e6d).wrapping_add(0x9b06);
        }
        o.extend_from_slice(&[0; 4]);
        o.drain(..4);
    }
    if compression & 1 != 0 {
        o = lnd(&o, n)?;
    }
    Ok(o)
}
pub fn ps2(d: &[u8], limit: usize) -> Result<Vec<u8>> {
    let packed = u32at(d, 36)? as usize;
    let n = u32at(d, 40)? as usize;
    ensure!(
        n.checked_add(48).is_some_and(|n| n <= limit),
        "PS2 output limit"
    );
    let v = u32at(d, 12)?;
    let key = ((v >> 24).wrapping_add(v >> 3)) as u8;
    let shift = (v >> 20) % 5 + 1;
    let encrypted = at(d, 48, packed)?;
    let plain: Vec<_> = encrypted
        .iter()
        .map(|b| (b.wrapping_sub(0x7c) ^ key).rotate_right(shift))
        .collect();
    let mut c = Cursor::new(&plain);
    let mut dict = [0u8; 2048];
    let mut pos = 0x7df;
    let mut o = at(d, 0, 48)?.to_vec();
    o[36..40].copy_from_slice(&(n as u32).to_le_bytes());
    let mut ctrl = 1u16;
    while o.len() < n + 48 {
        if ctrl == 1 {
            ctrl = c.u8()? as u16 | 0x100;
        }
        if ctrl & 1 != 0 {
            let b = c.u8()?;
            o.push(b);
            dict[pos] = b;
            pos = (pos + 1) & 2047;
        } else {
            let lo = c.u8()? as usize;
            let hi = c.u8()? as usize;
            let mut src = lo | ((hi & 0xe0) << 3);
            for _ in 0..((hi & 31) + 2).min(n + 48 - o.len()) {
                let b = dict[src];
                src = (src + 1) & 2047;
                o.push(b);
                dict[pos] = b;
                pos = (pos + 1) & 2047;
            }
        }
        ctrl >>= 1;
    }
    Ok(o)
}
pub fn acp(d: &[u8], limit: usize) -> Result<Vec<u8>> {
    let mut b = Bits::new(at(d, 4, d.len().saturating_sub(4))?);
    let n = b.get(32)? as usize;
    ensure!(n <= limit, "ACP output limit");
    let mut o = Vec::with_capacity(n);
    while o.len() < n {
        let mut dict = vec![[0usize; 2]; 35023];
        let mut marker = 259usize;
        let mut width = 9;
        let mut prev = b.get(width)? as usize;
        let mut first = prev;
        if prev == 256 {
            continue;
        }
        ensure!(prev < 256, "ACP first symbol");
        o.push(prev as u8);
        while o.len() < n {
            let code = b.get(width)? as usize;
            match code {
                257 => {
                    width += 1;
                    ensure!(width <= 16, "ACP code width");
                }
                256 | 258 => break,
                _ => {
                    let mut symbol = code;
                    let mut stack = Vec::new();
                    if symbol >= marker {
                        ensure!(symbol == marker, "ACP code outside dictionary");
                        symbol = prev;
                        stack.push(first as u8);
                    }
                    let mut steps = 0;
                    while symbol > 255 {
                        let node = dict.get(symbol).context("ACP dictionary index")?;
                        stack.push(node[1] as u8);
                        symbol = node[0];
                        steps += 1;
                        ensure!(steps < 35023, "ACP dictionary cycle");
                    }
                    stack.push(symbol as u8);
                    ensure!(
                        stack.len() <= n - o.len(),
                        "ACP output exceeds declared size"
                    );
                    o.extend(stack.into_iter().rev());
                    let entry = dict.get_mut(marker).context("ACP dictionary limit")?;
                    *entry = [prev, symbol];
                    marker += 1;
                    prev = code;
                    first = symbol;
                }
            }
        }
    }
    Ok(o)
}
pub fn bmr(d: &[u8], limit: usize) -> Result<Vec<u8>> {
    let step = at(d, 3, 1)?[0] as usize;
    let target = u32at(d, 4)? as usize;
    let key = u32at(d, 8)? as usize;
    let n = u32at(d, 12)? as usize;
    ensure!(n <= limit && target <= limit, "BMR output limit");
    let mut o = huffman(at(d, 20, d.len().saturating_sub(20))?, n)?;
    let mut alphabet = [0u8; 256];
    for (i, v) in alphabet.iter_mut().enumerate() {
        *v = i as u8;
    }
    for v in &mut o {
        let p = *v as usize;
        *v = alphabet[p];
        alphabet.copy_within(0..p, 1);
        alphabet[0] = *v;
    }
    if n > 0 {
        ensure!(key < n, "BMR transform key");
        let mut count = [0usize; 256];
        for b in &o {
            count[*b as usize] += 1;
        }
        for i in 1..256 {
            count[i] += count[i - 1];
        }
        let mut distribution = vec![0; n];
        for i in (0..n).rev() {
            let b = o[i] as usize;
            count[b] -= 1;
            distribution[count[b]] = i;
        }
        let mut pos = key;
        let mut plain = vec![0; n];
        for b in &mut plain {
            pos = distribution[pos];
            *b = o[pos];
        }
        o = plain;
    }
    if step > 0 {
        let mut c = Cursor::new(&o);
        let mut out = vec![0; target];
        for i in 0..step.min(target) {
            let mut pos = i;
            let mut prev = c.u8()?;
            out[pos] = prev;
            pos += step;
            while pos < target {
                let mut v = c.u8()?;
                out[pos] = v;
                pos += step;
                if v == prev {
                    let mut count = c.u8()? as usize;
                    if count & 128 != 0 {
                        count = c.u8()? as usize + ((count & 127) << 8) + 128;
                    }
                    for _ in 0..count {
                        if pos >= target {
                            break;
                        }
                        out[pos] = v;
                        pos += step;
                    }
                    if pos < target {
                        v = c.u8()?;
                        out[pos] = v;
                        pos += step;
                    }
                }
                prev = v;
            }
        }
        o = out;
    }
    Ok(o)
}
pub fn bse(d: &[u8]) -> Result<Vec<u8>> {
    let mut seed = u32at(d, 12)? as i32;
    let mut o = at(d, 16, 64)?.to_vec();
    let mut flags = [false; 64];
    let mut rand = || {
        let t = (seed.wrapping_mul(257) >> 8)
            .wrapping_add(seed.wrapping_mul(97))
            .wrapping_add(23) as u32
            ^ 0xa6cd9b75;
        seed = t.rotate_right(16) as i32;
        (seed as u32 & 0x7fff) as usize
    };
    for _ in 0..64 {
        let mut i = rand() % 64;
        while flags[i] {
            i = (i + 1) % 64;
        }
        let shift = rand() % 8;
        let key = rand();
        let v = o[i].wrapping_sub(rand() as u8);
        o[i] = if key & 1 != 0 {
            v.rotate_left(shift as u32)
        } else {
            v.rotate_right(shift as u32)
        };
        flags[i] = true;
    }
    ensure!(
        o.iter().fold(0u8, |a, b| a.wrapping_add(*b)) == d[10]
            && o.iter().fold(0u8, |a, b| a ^ b) == d[11],
        "BSE checksum mismatch"
    );
    o.extend_from_slice(&d[80..]);
    Ok(o)
}
fn update(key: &mut u32) -> u8 {
    let a = 0x4e35u32.wrapping_mul(*key & 0xffff);
    let b = 0x4e35u32.wrapping_mul(*key >> 16);
    let t = 0x15au32
        .wrapping_mul(*key)
        .wrapping_add(b)
        .wrapping_add(a >> 16);
    *key = (t << 16).wrapping_add(a & 0xffff).wrapping_add(1);
    t as u8
}
pub fn dsc(d: &[u8], cx: &crysirror_resource::DecodeContext) -> Result<(Vec<u8>, &'static str)> {
    let mut key = u32at(d, 16)?;
    let n = u32at(d, 20)? as usize;
    super::checked_size(n, cx)?;
    let lengths = at(d, 32, 512)?;
    let mut symbols = Vec::new();
    for (i, &v) in lengths.iter().enumerate() {
        let l = v.wrapping_sub(update(&mut key));
        if l > 0 {
            ensure!(l <= 32, "DSC Huffman code width");
            symbols.push((l, i));
        }
    }
    symbols.sort();
    ensure!(!symbols.is_empty() || n == 0, "DSC empty Huffman tree");
    let mut codes = Vec::new();
    let mut code = 0u64;
    let mut old = 0;
    for (l, s) in symbols {
        code <<= l - old;
        ensure!(code < (1u64 << l), "DSC oversubscribed Huffman tree");
        codes.push((l, code, s));
        code += 1;
        old = l;
    }
    let mut bits = Bits::new(at(d, 544, d.len().saturating_sub(544))?);
    let mut o = Vec::with_capacity(n);
    while o.len() < n {
        let mut value = 0u64;
        let mut found = None;
        for l in 1..=32 {
            value = (value << 1) | bits.get(1)? as u64;
            if let Some((_, _, s)) = codes.iter().find(|(len, c, _)| *len == l && *c == value) {
                found = Some(*s);
                break;
            }
        }
        let s = found.context("DSC invalid Huffman code")?;
        if s & 256 != 0 {
            let dist = bits.get(12)? as usize + 2;
            copy(&mut o, dist, (s & 255) + 2, n)?;
        } else {
            o.push(s as u8);
        }
    }
    if o.len() >= 16
        && u16at(&o, 0)? > 0
        && u16at(&o, 2)? > 0
        && matches!(o[4], 8 | 24 | 32)
        && o[5..16].iter().all(|v| *v == 0)
    {
        let w = u16at(&o, 0)? as u32;
        let h = u16at(&o, 2)? as u32;
        let channels = (o[4] / 8) as usize;
        ensure!(
            w as usize * h as usize * channels == o.len() - 16,
            "DSC image dimensions"
        );
        let png_size = crysirror_image::png_uncompressed_len(w, h)?;
        ensure!(
            png_size as u64 <= cx.limits.max_entry_size,
            "DSC PNG byte limit"
        );
        let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
        for p in o[16..].chunks_exact(channels) {
            match channels {
                1 => rgba.extend_from_slice(&[p[0], p[0], p[0], 255]),
                3 => rgba.extend_from_slice(&[p[2], p[1], p[0], 255]),
                4 => rgba.extend_from_slice(&[p[2], p[1], p[0], p[3]]),
                _ => unreachable!(),
            }
        }
        let png = crysirror_image::encode_png_uncompressed(&crysirror_image::ImageSurface {
            width: w,
            height: h,
            rgba,
        })?;
        return Ok((png, "png"));
    }
    let e = super::guess(&o);
    Ok((o, e))
}
