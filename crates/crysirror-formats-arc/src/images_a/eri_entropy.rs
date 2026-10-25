// SPDX-License-Identifier: GPL-3.0-or-later
// Safe translations of Entis entropy decoders in arc_unpacker, Copyright (C) 2016 rr-.
use super::common::Bits;
use anyhow::{ensure, Result};
const ROOT: usize = 512;
const NULL: usize = 32768;
const CODE: u32 = 0x80000000;
const ESC: u32 = 0x7fffffff;
#[derive(Clone, Copy, Default)]
struct Node {
    weight: u16,
    parent: usize,
    code: u32,
}
pub struct Huff {
    nodes: Vec<Node>,
    escape: usize,
    pointer: usize,
}
impl Huff {
    pub fn new() -> Self {
        let mut nodes = vec![Node::default(); 513];
        nodes[ROOT].parent = NULL;
        nodes[ROOT].code = NULL as u32;
        Self {
            nodes,
            escape: NULL,
            pointer: ROOT,
        }
    }
    fn recount(&mut self, parent: usize) {
        let c = self.nodes[parent].code as usize;
        self.nodes[parent].weight = self.nodes[c].weight.wrapping_add(self.nodes[c + 1].weight);
    }
    fn normalize(&mut self, mut entry: usize) {
        while entry < ROOT {
            let mut swap = entry + 1;
            let weight = self.nodes[entry].weight;
            while swap < ROOT && self.nodes[swap].weight < weight {
                swap += 1;
            }
            swap -= 1;
            if entry == swap {
                entry = self.nodes[entry].parent;
                self.recount(entry);
                continue;
            }
            for (from, to) in [(entry, swap), (swap, entry)] {
                let code = self.nodes[from].code;
                if code & CODE == 0 {
                    self.nodes[code as usize].parent = to;
                    self.nodes[code as usize + 1].parent = to;
                } else if code & !CODE == ESC {
                    self.escape = to;
                }
            }
            let (ep, sp) = (self.nodes[entry].parent, self.nodes[swap].parent);
            self.nodes.swap(entry, swap);
            self.nodes[swap].parent = sp;
            self.nodes[entry].parent = ep;
            self.recount(sp);
            entry = sp;
        }
    }
    fn increase(&mut self, entry: usize) {
        self.nodes[entry].weight += 1;
        self.normalize(entry);
        if self.nodes[ROOT].weight >= 0x4000 {
            self.half();
        }
    }
    fn add(&mut self, code: u32) {
        if self.pointer > 0 {
            self.pointer -= 2;
            let i = self.pointer;
            self.nodes[i].weight = 1;
            self.nodes[i].code = CODE | code;
            if self.nodes[ROOT].code != NULL as u32 {
                self.nodes[i + 1] = self.nodes[i + 2];
                if self.nodes[i + 1].code == CODE | ESC {
                    self.escape = i + 1;
                }
                self.nodes[i + 2].weight = self.nodes[i].weight + self.nodes[i + 1].weight;
                self.nodes[i + 2].parent = self.nodes[i + 1].parent;
                self.nodes[i + 2].code = i as u32;
                self.nodes[i].parent = i + 2;
                self.nodes[i + 1].parent = i + 2;
                self.normalize(i + 2);
            } else {
                self.nodes[i].parent = ROOT;
                self.escape = i + 1;
                self.nodes[i + 1] = Node {
                    weight: 1,
                    parent: ROOT,
                    code: CODE | ESC,
                };
                self.nodes[ROOT].weight = 2;
                self.nodes[ROOT].code = i as u32;
            }
        } else {
            let i = if self.nodes[0].code == CODE | ESC {
                1
            } else {
                0
            };
            self.nodes[i].code = CODE | code;
        }
    }
    fn half(&mut self) {
        let mut next = ROOT as isize;
        for i in (self.pointer..ROOT).rev() {
            if self.nodes[i].code & CODE != 0 {
                self.nodes[i].weight = (self.nodes[i].weight + 1) >> 1;
                self.nodes[next as usize] = self.nodes[i];
                next -= 1;
            }
        }
        next += 1;
        let mut i = self.pointer;
        loop {
            self.nodes[i] = self.nodes[next as usize];
            self.nodes[i + 1] = self.nodes[next as usize + 1];
            next += 2;
            for child_at in [i, i + 1] {
                let code = self.nodes[child_at].code;
                if code & CODE == 0 {
                    self.nodes[code as usize].parent = child_at;
                    self.nodes[code as usize + 1].parent = child_at;
                } else if code & !CODE == ESC {
                    self.escape = child_at;
                }
            }
            let weight = self.nodes[i].weight + self.nodes[i + 1].weight;
            if next <= ROOT as isize {
                let mut j = next as usize;
                loop {
                    if weight <= self.nodes[j].weight {
                        self.nodes[j - 1].weight = weight;
                        self.nodes[j - 1].code = i as u32;
                        break;
                    }
                    self.nodes[j - 1] = self.nodes[j];
                    j += 1;
                    if j > ROOT {
                        self.nodes[ROOT].weight = weight;
                        self.nodes[ROOT].code = i as u32;
                        break;
                    }
                }
                next -= 1;
            } else {
                self.nodes[ROOT] = Node {
                    weight,
                    parent: NULL,
                    code: i as u32,
                };
                self.nodes[i].parent = ROOT;
                self.nodes[i + 1].parent = ROOT;
                break;
            }
            i += 2;
        }
    }
    pub fn read(&mut self, b: &mut Bits<'_>, size: bool) -> Result<u32> {
        if self.escape != NULL {
            let mut entry = ROOT;
            let mut code = self.nodes[ROOT].code;
            while code & CODE == 0 {
                entry = code as usize + b.get(1)? as usize;
                ensure!(entry <= ROOT, "invalid ERI Huffman node");
                code = self.nodes[entry].code;
            }
            self.increase(entry);
            let raw = code & !CODE;
            if raw != ESC {
                return Ok(raw);
            }
        }
        let code = if size { gamma(b)? } else { b.get(8)? };
        self.add(code);
        Ok(code)
    }
}
pub fn gamma(b: &mut Bits<'_>) -> Result<u32> {
    if b.get(1)? == 0 {
        return Ok(1);
    }
    let (mut base, mut code) = (2u32, 0u32);
    loop {
        code = (code << 1) | b.get(1)?;
        if b.get(1)? == 0 {
            return Ok(code + base);
        }
        ensure!(base < 1 << 30, "ERI gamma overflow");
        base <<= 1;
    }
}
#[derive(Clone)]
struct Model {
    symbols: Vec<(u32, i16)>,
    total: u32,
}
impl Model {
    fn new() -> Self {
        Self {
            symbols: (0..257)
                .map(|i| (1, if i == 256 { -1 } else { i as i16 }))
                .collect(),
            total: 257,
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
                v.0 = v.0.div_ceil(2);
                self.total += v.0;
            }
        }
    }
}
fn arithmetic(b: &mut Bits<'_>, code: &mut u32, aug: &mut u32, m: &mut Model) -> Result<i16> {
    ensure!(*aug != 0, "ERI empty arithmetic range");
    let mut acc = code.wrapping_mul(m.total) / *aug;
    ensure!(acc < 8192, "ERI arithmetic symbol out of range");
    let (mut i, mut fs) = (0, 0);
    while i < m.symbols.len() && acc >= m.symbols[i].0 {
        acc -= m.symbols[i].0;
        fs += m.symbols[i].0;
        i += 1;
    }
    ensure!(i < m.symbols.len(), "ERI arithmetic escape");
    let (freq, symbol) = m.symbols[i];
    *code = code.wrapping_sub((*aug * fs).div_ceil(m.total));
    *aug = *aug * freq / m.total;
    ensure!(*aug != 0, "ERI empty arithmetic range");
    while *aug & 0x8000 == 0 {
        *code = code.wrapping_shl(1) | b.get(1)?;
        *aug <<= 1;
    }
    *code &= 65535;
    m.increase(i);
    Ok(symbol)
}
pub struct Entropy<'a> {
    pub bits: Bits<'a>,
    architecture: i32,
    zero: bool,
    available: usize,
    huff: Vec<Huff>,
    last: usize,
    models: Vec<Model>,
    rle: Model,
    op_model: Model,
    code: u32,
    aug: u32,
}
impl<'a> Entropy<'a> {
    pub fn new(bytes: &'a [u8], architecture: i32) -> Result<Self> {
        ensure!(
            matches!(architecture, -1 | -4 | -16),
            "unsupported ERI entropy architecture"
        );
        Ok(Self {
            bits: Bits::new(bytes, true),
            architecture,
            zero: false,
            available: 0,
            huff: Vec::new(),
            last: 0,
            models: Vec::new(),
            rle: Model::new(),
            op_model: Model::new(),
            code: 0,
            aug: 0,
        })
    }
    pub fn reset(&mut self) -> Result<()> {
        self.available = 0;
        self.last = 0;
        match self.architecture {
            -1 => self.zero = self.bits.get(1)? == 0,
            -4 => self.huff = (0..257).map(|_| Huff::new()).collect(),
            -16 => {
                self.code = self.bits.get(32)?;
                self.aug = 65535;
                self.models = (0..256).map(|_| Model::new()).collect();
            }
            _ => unreachable!(),
        }
        Ok(())
    }
    pub fn operation(&mut self) -> Result<u8> {
        let s = arithmetic(
            &mut self.bits,
            &mut self.code,
            &mut self.aug,
            &mut self.op_model,
        )?;
        ensure!(s >= 0, "ERI escaped transform opcode");
        Ok(s as u8)
    }
    pub fn decode(&mut self, n: usize) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(n);
        if self.architecture == -1 {
            while out.len() < n {
                if self.available == 0 {
                    self.available = gamma(&mut self.bits)? as usize;
                }
                let count = self.available.min(n - out.len());
                self.available -= count;
                for _ in 0..count {
                    let val = if self.zero {
                        0
                    } else {
                        let sign = self.bits.get(1)? != 0;
                        let v = gamma(&mut self.bits)? as u8;
                        if sign {
                            v.wrapping_neg()
                        } else {
                            v
                        }
                    };
                    out.push(val);
                }
                if self.available == 0 {
                    self.zero = !self.zero;
                }
            }
            return Ok(out);
        }
        while out.len() < n {
            if self.available > 0 {
                let k = self.available.min(n - out.len());
                self.available -= k;
                out.resize(out.len() + k, 0);
                continue;
            }
            let symbol = if self.architecture == -4 {
                self.huff[self.last].read(&mut self.bits, false)? as i16
            } else {
                arithmetic(
                    &mut self.bits,
                    &mut self.code,
                    &mut self.aug,
                    &mut self.models[self.last],
                )?
            };
            ensure!(symbol >= 0 && symbol <= 255, "ERI entropy escape");
            out.push(symbol as u8);
            if symbol == 0 {
                self.available = if self.architecture == -4 {
                    let size = self.huff[256].read(&mut self.bits, true)?;
                    ensure!(size > 0, "ERI zero Huffman run");
                    size as usize - 1
                } else {
                    let n =
                        arithmetic(&mut self.bits, &mut self.code, &mut self.aug, &mut self.rle)?;
                    ensure!(n >= 0, "ERI run escape");
                    n as usize
                };
            }
            self.last = symbol as usize;
        }
        Ok(out)
    }
}
