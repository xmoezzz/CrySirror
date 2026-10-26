// SPDX-License-Identifier: GPL-3.0-or-later
// Entis ERISA-N / BSHF codecs translated from arc_unpacker, Copyright (C) 2016 rr-.
use super::*;
use crate::images_a::common::Bits;
#[derive(Clone)]
struct Model {
    symbols: Vec<(u32, i16)>,
    total: u32,
    sub: Vec<(u16, i16)>,
}
impl Model {
    fn new() -> Self {
        Self {
            symbols: (0..257)
                .map(|i| (1, if i == 256 { -1 } else { i as i16 }))
                .collect(),
            total: 257,
            sub: vec![(0, -1); 128],
        }
    }
    fn increase(&mut self, mut i: usize) {
        self.symbols[i].0 += 1;
        let v = self.symbols[i];
        while i > 0 && self.symbols[i - 1].0 < v.0 {
            self.symbols[i] = self.symbols[i - 1];
            i -= 1;
        }
        self.symbols[i] = v;
        self.total += 1;
        if self.total >= 8192 {
            self.total = 0;
            for v in &mut self.symbols {
                v.0 = (v.0 + 1) >> 1;
                self.total += v.0;
            }
            for v in &mut self.sub {
                v.0 >>= 1;
            }
        }
    }
    fn add(&mut self, symbol: i16) -> Result<()> {
        ensure!(self.symbols.len() < 257, "ERISA-N symbol model overflow");
        self.symbols.push((1, symbol));
        self.total += 1;
        Ok(())
    }
    fn from_parent(parent: &Self) -> Self {
        let mut symbols = Vec::new();
        let mut total = 1;
        for &(count, symbol) in &parent.symbols {
            let n = count >> 4;
            if n > 0 && symbol != -1 {
                symbols.push((n, symbol));
                total += n;
            }
        }
        symbols.push((1, -1));
        Self {
            symbols,
            total,
            sub: vec![(0, -1); 128],
        }
    }
}
struct Arithmetic<'a> {
    bits: Bits<'a>,
    code: u32,
    aug: u32,
}
impl<'a> Arithmetic<'a> {
    fn new(b: &'a [u8]) -> Result<Self> {
        let mut bits = Bits::new(b, true);
        let code = bits.get(32)?;
        Ok(Self {
            bits,
            code,
            aug: 65535,
        })
    }
    fn index(&mut self, m: &Model) -> Result<usize> {
        ensure!(
            self.aug != 0 && m.total > 0,
            "ERISA-N empty arithmetic range"
        );
        let mut acc = self.code.wrapping_mul(m.total) / self.aug;
        ensure!(acc < 8192, "ERISA-N probability range");
        let (mut index, mut fs) = (0, 0u32);
        while index < m.symbols.len() && acc >= m.symbols[index].0 {
            acc -= m.symbols[index].0;
            fs += m.symbols[index].0;
            index += 1;
        }
        let &(freq, _) = m.symbols.get(index).context("ERISA-N probability symbol")?;
        self.code = self
            .code
            .wrapping_sub((self.aug * fs + m.total - 1) / m.total);
        self.aug = self.aug * freq / m.total;
        ensure!(self.aug != 0, "ERISA-N empty arithmetic augend");
        while self.aug & 0x8000 == 0 {
            self.code = (self.code << 1) | self.bits.get(1)?;
            self.aug <<= 1;
        }
        self.code &= 65535;
        Ok(index)
    }
    fn read(&mut self, m: &mut Model) -> Result<i16> {
        let i = self.index(m)?;
        let symbol = m.symbols[i].1;
        m.increase(i);
        Ok(symbol)
    }
}
pub(super) fn erisan(b: &[u8], size: usize) -> Result<Vec<u8>> {
    let mut arithmetic = Arithmetic::new(b)?;
    let mut models = vec![Model::new()];
    let mut phrase_len = Model::new();
    let mut phrase_index = Model::new();
    let mut run_len = Model::new();
    let mut last = [0u8; 4];
    let mut last_pos = 0usize;
    let mut buffer = vec![0u8; 65536];
    let mut buffer_pos = 0usize;
    let mut lookup = vec![vec![0usize; 256]; 256];
    let mut lookup_pos = [0usize; 256];
    let mut out = Vec::with_capacity(size);
    let shift = [1, 3, 4, 5];
    let threshold = [1, 8, 16, 32];
    let mut remaining = 0usize;
    let mut next = None;
    while out.len() < size {
        if remaining > 0 {
            let n = remaining.min(size - out.len());
            let mut last_symbol = buffer[buffer_pos.wrapping_sub(1) & 65535];
            for _ in 0..n {
                let symbol = if let Some(i) = next {
                    let symbol = buffer[i & 65535];
                    next = Some((i + 1) & 65535);
                    symbol
                } else {
                    last_symbol
                };
                last_symbol = symbol;
                let s = symbol as usize;
                lookup[s][lookup_pos[s]] = buffer_pos;
                lookup_pos[s] = (lookup_pos[s] + 1) & 255;
                last[last_pos] = symbol;
                last_pos = (last_pos + 1) & 3;
                buffer[buffer_pos] = symbol;
                buffer_pos = (buffer_pos + 1) & 65535;
                out.push(symbol);
            }
            remaining -= n;
            continue;
        }
        let mut model = 0usize;
        let mut degree = 0;
        while degree < 4 {
            let symbol = last[last_pos.wrapping_sub(1 + degree) & 3] as usize >> shift[degree];
            let child = models[model].sub[symbol].1;
            if child < 0 {
                break;
            }
            let child = child as usize + 1;
            ensure!(child < models.len(), "ERISA-N model child outside pool");
            model = child;
            degree += 1;
        }
        let mut symbol = arithmetic.read(&mut models[model])?;
        let mut nemesis = false;
        if symbol == -1 {
            if model != 0 {
                symbol = arithmetic.read(&mut models[0])?;
                if symbol != -1 {
                    models[model].add(symbol)?;
                } else {
                    nemesis = true;
                }
            } else {
                nemesis = true;
            }
        }
        if nemesis {
            let phrase = arithmetic.read(&mut phrase_index)?;
            ensure!(phrase >= 0, "ERISA-N end marker before declared size");
            let n = if phrase != 0 {
                arithmetic.read(&mut phrase_len)?
            } else {
                arithmetic.read(&mut run_len)?
            };
            ensure!(n > 0, "ERISA-N zero/invalid phrase length");
            remaining = n as usize;
            let symbol = buffer[buffer_pos.wrapping_sub(1) & 65535];
            if phrase == 0 {
                next = None;
            } else {
                let p = lookup[symbol as usize]
                    [lookup_pos[symbol as usize].wrapping_sub(phrase as usize) & 255];
                ensure!(
                    buffer[p & 65535] == symbol,
                    "ERISA-N phrase symbol mismatch"
                );
                next = Some((p + 1) & 65535);
            }
            continue;
        }
        ensure!(
            (0..256).contains(&symbol),
            "ERISA-N literal outside byte range"
        );
        let symbol = symbol as u8;
        last[last_pos] = symbol;
        last_pos = (last_pos + 1) & 3;
        lookup[symbol as usize][lookup_pos[symbol as usize]] = buffer_pos;
        lookup_pos[symbol as usize] = (lookup_pos[symbol as usize] + 1) & 255;
        buffer[buffer_pos] = symbol;
        buffer_pos = (buffer_pos + 1) & 65535;
        out.push(symbol);
        if models.len() > 2048 || degree >= 4 {
            continue;
        }
        let index = symbol as usize >> shift[degree];
        let sub = &mut models[model].sub[index];
        sub.0 = sub.0.wrapping_add(1);
        if sub.0 < threshold[degree] {
            continue;
        }
        let parent = model;
        model = 0;
        let mut depth = 0;
        let mut symbol_index = 0;
        while depth <= degree {
            symbol_index = last[last_pos.wrapping_sub(1 + depth) & 3] as usize >> shift[depth];
            let child = models[model].sub[symbol_index].1;
            if child < 0 {
                break;
            }
            model = child as usize + 1;
            ensure!(model < models.len(), "ERISA-N model outside pool");
            depth += 1;
        }
        if depth > degree || models[model].sub[symbol_index].1 >= 0 {
            continue;
        }
        let new_model = Model::from_parent(&models[parent]);
        models[model].sub[symbol_index].1 = (models.len() - 1) as i16;
        models.push(new_model);
    }
    Ok(out)
}
pub(super) fn bshf(b: &[u8], size: usize, key: &[u8]) -> Result<Vec<u8>> {
    ensure!(!key.is_empty(), "NOA BSHF requires caller key");
    let mut pass = key.to_vec();
    if pass.len() < 32 {
        pass.push(0x1b);
        let size = pass.len();
        for i in size..32 {
            pass.push(pass[i % size].wrapping_add(pass[i - 1]));
        }
    }
    let mut out = Vec::with_capacity(size);
    let mut key_offset = 0;
    for block in b.chunks_exact(32) {
        let mut dst = [0u8; 32];
        let mut mask = [0u8; 32];
        let mut pos = key_offset % pass.len();
        key_offset = pos + 1;
        let mut bit = 0u8;
        for i in 0..256 {
            bit = bit.wrapping_add(pass[pos]);
            pos = (pos + 1) % pass.len();
            let mut offset = (bit >> 3) as usize;
            let mut m = 0x80 >> (bit & 7);
            let mut steps = 0;
            while mask[offset] == 255 {
                bit = bit.wrapping_add(8);
                offset = (bit >> 3) as usize;
                steps += 1;
                ensure!(steps < 32, "BSHF full bit mask");
            }
            while mask[offset] & m != 0 {
                bit = bit.wrapping_add(1);
                m >>= 1;
                if m == 0 {
                    bit = bit.wrapping_add(8);
                    offset = (bit >> 3) as usize;
                    m = 128;
                }
                steps += 1;
                ensure!(steps < 288, "BSHF bit traversal stalled");
            }
            mask[offset] |= m;
            if block[i >> 3] & (128 >> (i & 7)) != 0 {
                dst[offset] |= m;
            }
        }
        let n = (size - out.len()).min(32);
        out.extend(&dst[..n]);
        if out.len() == size {
            return Ok(out);
        }
    }
    ensure!(out.len() == size, "truncated NOA BSHF block");
    Ok(out)
}
