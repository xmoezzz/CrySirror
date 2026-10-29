// Portions adapted from xmoezzz/xp3-brute, revision c82cb14302138125640478556723705aa720007b.
// Copyright (c) xp3-brute contributors; modifications CrySirror contributors.
// MIT License: see NOTICE-MIT in this directory.
//! Parameterized CXDEC content-filter core (upstream `src/cxdec_classic.rs`).
//!
//! The engine knows nothing about game names or PE layout: a recognizer
//! supplies a complete [`CxdecProfile`] recovered from the user's own modules.
//! Generated x86 is never executed; the 128 expression programs are rebuilt
//! and evaluated in Rust. No published per-title parameter table is carried.
//!
//! CrySirror delta: the hash-derived first-eight-byte wrapper takes its two
//! XOR words from the profile. Upstream hard-codes `0x55555555/0xaaaaaaaa`;
//! some modules load different words from `.rdata` during plugin link, which
//! [`super::recover`] recovers statically.
use anyhow::{bail, ensure, Result};

pub const CONTROL_BLOCK_SIZE: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum GeneratorKind {
    Classic,
    /// Two-state xorshift generator with a module-recovered second seed.
    Cabbage {
        random_seed: u32,
    },
}

/// Hash-derived XOR over logical file bytes 0..8. `lo`/`hi` are the words
/// XORed into the low/high halves of the expanded 64-bit key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Prefix8 {
    pub lo: u32,
    pub hi: u32,
}
impl Prefix8 {
    /// Algorithm default observed in the earliest Prefix8 modules.
    pub const DEFAULT: Self = Self {
        lo: 0x5555_5555,
        hi: 0xaaaa_aaaa,
    };
    pub fn key(self, hash: u32) -> u64 {
        let lo = hash ^ self.lo;
        let mut hi = hash.wrapping_shl(13) ^ hash;
        hi ^= hi.wrapping_shr(17);
        hi ^= hi.wrapping_shl(5) ^ self.hi;
        (u64::from(hi) << 32) | u64::from(lo)
    }
    pub fn apply(self, file_offset: u64, hash: u32, data: &mut [u8]) {
        if data.is_empty() || file_offset >= 8 {
            return;
        }
        let key = self.key(hash);
        let start = file_offset as usize;
        let count = data.len().min(8 - start);
        for (i, b) in data[..count].iter_mut().enumerate() {
            *b ^= (key >> ((start + i) * 8)) as u8;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CxdecProfile {
    pub mask: u32,
    pub offset: u32,
    pub prolog_order: [u8; 3],
    pub even_branch_order: [u8; 8],
    pub odd_branch_order: [u8; 6],
    /// Raw 4096-byte control table as seen by the generated content program.
    pub control_block: Vec<u8>,
    pub generator: GeneratorKind,
    pub prefix8: Option<Prefix8>,
}
impl CxdecProfile {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.control_block.len() == CONTROL_BLOCK_SIZE,
            "CXDEC control block must be 4096 bytes"
        );
        ensure!(permutation(&self.prolog_order), "CXDEC prolog order");
        ensure!(permutation(&self.even_branch_order), "CXDEC even order");
        ensure!(permutation(&self.odd_branch_order), "CXDEC odd order");
        Ok(())
    }
}
pub(super) fn permutation(v: &[u8]) -> bool {
    let mut seen = vec![false; v.len()];
    v.iter().all(|&x| {
        let ok = seen.get(x as usize).is_some_and(|s| !s);
        if ok {
            seen[x as usize] = true;
        }
        ok
    })
}

/// Native evaluator: owned recovered data and 128 expression programs only.
#[derive(Clone, Debug)]
pub struct CxdecEngine {
    profile: CxdecProfile,
    words: Box<[u32; 1024]>,
    lanes: Vec<Expr>,
}
impl CxdecEngine {
    pub fn new(profile: CxdecProfile) -> Result<Self> {
        profile.validate()?;
        let mut words = Box::new([0u32; 1024]);
        for (w, c) in words.iter_mut().zip(profile.control_block.chunks_exact(4)) {
            *w = u32::from_le_bytes(c.try_into().unwrap());
        }
        let lanes = (0..128)
            .map(|l| build_lane(l, &profile))
            .collect::<Result<_>>()?;
        Ok(Self {
            profile,
            words,
            lanes,
        })
    }
    pub fn profile(&self) -> &CxdecProfile {
        &self.profile
    }
    pub fn evaluate_lane(&self, lane: u8, parameter: u32) -> u32 {
        self.eval(&self.lanes[(lane & 0x7f) as usize], parameter)
    }
    /// XOR-symmetric; `file_offset` is the logical (decompressed) offset.
    pub fn apply(&self, file_offset: u64, hash: u32, data: &mut [u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        ensure!(
            file_offset
                .checked_add(data.len() as u64)
                .is_some_and(|end| end <= 1 << 32),
            "CXDEC uses 32-bit logical entry offsets"
        );
        if let Some(p) = self.profile.prefix8 {
            p.apply(file_offset, hash, data);
        }
        let offset = file_offset as u32;
        let boundary = (hash & self.profile.mask).wrapping_add(self.profile.offset);
        let first = if offset < boundary {
            data.len().min(boundary.wrapping_sub(offset) as usize)
        } else {
            0
        };
        if first != 0 {
            self.region(hash, offset, &mut data[..first]);
        }
        if first != data.len() {
            self.region(
                (hash >> 16) ^ hash,
                offset.wrapping_add(first as u32),
                &mut data[first..],
            );
        }
        Ok(())
    }
    fn region(&self, hash: u32, offset: u32, data: &mut [u8]) {
        let lane = &self.lanes[(hash & 0x7f) as usize];
        let seed = hash >> 7;
        let result = self.eval(lane, seed);
        let inverse = self.eval(lane, !seed);
        let high = inverse >> 16;
        let mut low = inverse & 0xffff;
        if high == low {
            low = low.wrapping_add(1);
        }
        let body = (result as u8).max(1);
        for b in data.iter_mut() {
            *b ^= body;
        }
        let end = u64::from(offset) + data.len() as u64;
        for (pos, value) in [(low, (result >> 16) as u8), (high, (result >> 8) as u8)] {
            let pos = u64::from(pos);
            if pos >= u64::from(offset) && pos < end {
                data[(pos - u64::from(offset)) as usize] ^= value;
            }
        }
    }
    fn eval(&self, e: &Expr, seed: u32) -> u32 {
        match e {
            Expr::Seed => seed,
            Expr::Imm(v) => *v,
            Expr::Table(i) => self.words[*i as usize],
            Expr::Unary(op, inner) => {
                let v = self.eval(inner, seed);
                match *op {
                    Unary::Not => !v,
                    Unary::Dec => v.wrapping_sub(1),
                    Unary::Neg => 0u32.wrapping_sub(v),
                    Unary::Inc => v.wrapping_add(1),
                    Unary::Table => self.words[(v & 0x3ff) as usize],
                    Unary::Interlace => ((v & 0xaaaa_aaaa) >> 1) | ((v & 0x5555_5555) << 1),
                    Unary::Xor(k) => v ^ k,
                    Unary::Add(k) => v.wrapping_add(k),
                    Unary::Sub(k) => v.wrapping_sub(k),
                }
            }
            Expr::Binary(op, a, b) => {
                let ebx = self.eval(a, seed);
                let eax = self.eval(b, seed);
                match op {
                    Binary::Shr => eax.wrapping_shr(ebx & 0x0f),
                    Binary::Shl => eax.wrapping_shl(ebx & 0x0f),
                    Binary::Add => eax.wrapping_add(ebx),
                    Binary::ReverseSub => ebx.wrapping_sub(eax),
                    Binary::Mul => eax.wrapping_mul(ebx),
                    Binary::Sub => eax.wrapping_sub(ebx),
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
enum Expr {
    Seed,
    Imm(u32),
    Table(u16),
    Unary(Unary, Box<Expr>),
    Binary(Binary, Box<Expr>, Box<Expr>),
}
#[derive(Clone, Copy, Debug)]
enum Unary {
    Not,
    Dec,
    Neg,
    Inc,
    Table,
    Interlace,
    Xor(u32),
    Add(u32),
    Sub(u32),
}
#[derive(Clone, Copy, Debug)]
enum Binary {
    Shr,
    Shl,
    Add,
    ReverseSub,
    Mul,
    Sub,
}

struct Rng {
    seed: u32,
    kind: GeneratorKind,
    random: u32,
}
impl Rng {
    fn next(&mut self) -> u32 {
        match self.kind {
            GeneratorKind::Classic => {
                let old = self.seed;
                self.seed = 0x41c6_4e6du32.wrapping_mul(old).wrapping_add(0x3039);
                self.seed ^ old.wrapping_shl(16) ^ old.wrapping_shr(16)
            }
            GeneratorKind::Cabbage { .. } => {
                let mut s = self.seed ^ self.seed.wrapping_shl(17);
                s ^= s.wrapping_shl(18) | s.wrapping_shr(15);
                self.seed = !s;
                let mut r = self.random ^ self.random.wrapping_shl(13);
                r ^= r.wrapping_shr(17);
                self.random = r ^ r.wrapping_shl(5);
                self.seed ^ self.random
            }
        }
    }
}

/// Mirrors the native builder's 128-byte code budget so the same random
/// stream (including byte-budget retries) produces the same expression.
struct Builder<'a> {
    rng: &'a mut Rng,
    p: &'a CxdecProfile,
    len: usize,
}
impl Builder<'_> {
    fn reserve(&mut self, n: usize) -> bool {
        match self.len.checked_add(n) {
            Some(next) if next <= 128 => {
                self.len = next;
                true
            }
            _ => false,
        }
    }
    fn prolog(&mut self) -> Option<Expr> {
        match self.p.prolog_order[(self.rng.next() % 3) as usize] {
            0 => {
                if !self.reserve(1) {
                    return None;
                }
                let v = self.rng.next();
                self.reserve(4).then_some(Expr::Imm(v))
            }
            1 => self.reserve(2).then_some(Expr::Seed),
            2 => {
                if !self.reserve(5) || !self.reserve(2) {
                    return None;
                }
                let i = (self.rng.next() as u16) & 0x3ff;
                self.reserve(4).then_some(Expr::Table(i))
            }
            _ => None,
        }
    }
    fn child(&mut self, stage: u32) -> Option<Expr> {
        if self.rng.next() & 1 != 0 {
            self.stage1(stage - 1)
        } else {
            self.stage0(stage - 1)
        }
    }
    fn stage0(&mut self, stage: u32) -> Option<Expr> {
        if stage == 1 {
            return self.prolog();
        }
        let child = self.child(stage)?;
        let op = match self.p.even_branch_order[(self.rng.next() & 7) as usize] {
            0 => self.reserve(2).then_some(Unary::Not),
            1 => self.reserve(1).then_some(Unary::Dec),
            2 => self.reserve(2).then_some(Unary::Neg),
            3 => self.reserve(1).then_some(Unary::Inc),
            4 => (self.reserve(5) && self.reserve(1) && self.reserve(4) && self.reserve(3))
                .then_some(Unary::Table),
            5 => self
                .reserve(1 + 2 + 6 + 5 + 2 + 2 + 2 + 1)
                .then_some(Unary::Interlace),
            6 => {
                if !self.reserve(1) {
                    return None;
                }
                let v = self.rng.next();
                self.reserve(4).then_some(Unary::Xor(v))
            }
            7 => {
                let add = self.rng.next() & 1 != 0;
                if !self.reserve(1) {
                    return None;
                }
                let v = self.rng.next();
                self.reserve(4)
                    .then_some(if add { Unary::Add(v) } else { Unary::Sub(v) })
            }
            _ => None,
        }?;
        Some(Expr::Unary(op, Box::new(child)))
    }
    fn stage1(&mut self, stage: u32) -> Option<Expr> {
        if stage == 1 {
            return self.prolog();
        }
        if !self.reserve(1) {
            return None;
        }
        let first = self.child(stage)?;
        if !self.reserve(2) {
            return None;
        }
        let second = self.child(stage)?;
        let op = match self.p.odd_branch_order[(self.rng.next() % 6) as usize] {
            0 => self.reserve(9).then_some(Binary::Shr),
            1 => self.reserve(9).then_some(Binary::Shl),
            2 => self.reserve(2).then_some(Binary::Add),
            3 => self.reserve(4).then_some(Binary::ReverseSub),
            4 => self.reserve(3).then_some(Binary::Mul),
            5 => self.reserve(2).then_some(Binary::Sub),
            _ => None,
        }?;
        if !self.reserve(1) {
            return None;
        }
        Some(Expr::Binary(op, Box::new(first), Box::new(second)))
    }
}

fn build_lane(lane: u32, p: &CxdecProfile) -> Result<Expr> {
    let random = match p.generator {
        GeneratorKind::Classic => 0,
        GeneratorKind::Cabbage { random_seed } => random_seed,
    };
    let mut rng = Rng {
        seed: lane,
        kind: p.generator,
        random,
    };
    for stage in (1..=5u32).rev() {
        let mut b = Builder {
            rng: &mut rng,
            p,
            len: 9,
        };
        if let Some(e) = b.stage1(stage) {
            if b.reserve(6) {
                return Ok(e);
            }
        }
    }
    bail!("CXDEC lane {lane} exceeded the 128-byte generator budget")
}

#[cfg(test)]
mod tests {
    use super::*;
    // Synthetic parameters only: identity orders and an arithmetic table.
    fn profile(generator: GeneratorKind, prefix8: Option<Prefix8>) -> CxdecProfile {
        let mut control_block = vec![0u8; CONTROL_BLOCK_SIZE];
        for (i, w) in control_block.chunks_exact_mut(4).enumerate() {
            w.copy_from_slice(&(i as u32).wrapping_mul(0x9e37_79b9).to_le_bytes());
        }
        CxdecProfile {
            mask: 0x1a3,
            offset: 0x0b6,
            prolog_order: [2, 0, 1],
            even_branch_order: [0, 7, 5, 6, 3, 1, 4, 2],
            odd_branch_order: [4, 3, 2, 1, 5, 0],
            control_block,
            generator,
            prefix8,
        }
    }
    #[test]
    fn cabbage_prng_matches_upstream_reference_vectors() {
        // Fixed vectors from upstream's independent reference test.
        for (seed, random, expected) in [
            (
                0,
                0,
                [
                    0xffff_ffffu32,
                    0x0002_0003,
                    0xfff7_fff4,
                    0x0030_0034,
                    0xff77_ff7b,
                    0x0392_0387,
                ],
            ),
            (
                0x1234_5678,
                0x9abc_def0,
                [
                    0x35e3_c26e,
                    0xee55_4882,
                    0xed67_c7b3,
                    0x7516_9cb0,
                    0x15d0_9850,
                    0xa844_8eeb,
                ],
            ),
        ] {
            let mut rng = Rng {
                seed,
                kind: GeneratorKind::Cabbage {
                    random_seed: random,
                },
                random,
            };
            assert_eq!(expected.map(|_| rng.next()), expected);
        }
    }
    #[test]
    fn cabbage_lane_values_match_upstream_reference_vectors() {
        let mut p = profile(
            GeneratorKind::Cabbage {
                random_seed: 0x9abc_def0,
            },
            None,
        );
        p.prolog_order = [1, 0, 2];
        p.even_branch_order = [7, 6, 5, 1, 0, 3, 4, 2];
        p.odd_branch_order = [3, 2, 1, 4, 5, 0];
        let e = CxdecEngine::new(p).unwrap();
        for (lane, x, want) in [
            (0u8, 0u32, 0x177e_71fbu32),
            (1, 0x0012_3456, 0x3a49_03ff),
            (17, 0x89ab_cdef, 0xa93b_fd53),
            (63, 0x0102_0304, 0x00bd_6d56),
            (127, 0xffff_ffff, 0x3efa_bb70),
        ] {
            assert_eq!(e.evaluate_lane(lane, x), want);
        }
    }
    #[test]
    fn partial_buffers_match_whole_buffer_and_round_trip() {
        for g in [
            GeneratorKind::Classic,
            GeneratorKind::Cabbage {
                random_seed: 0x1357_9bdf,
            },
        ] {
            for pre in [
                None,
                Some(Prefix8::DEFAULT),
                Some(Prefix8 {
                    lo: 0x0bad_f00d,
                    hi: 0x1234_5678,
                }),
            ] {
                let e = CxdecEngine::new(profile(g, pre)).unwrap();
                let hash = 0x8a4b_192e;
                let plain: Vec<u8> = (0..8192).map(|i| i as u8).collect();
                let mut whole = plain.clone();
                e.apply(0, hash, &mut whole).unwrap();
                let mut pieces = plain.clone();
                for (o, n) in [
                    (0usize, 3usize),
                    (3, 10),
                    (13, 1024),
                    (1037, 2000),
                    (3037, 5155),
                ] {
                    e.apply(o as u64, hash, &mut pieces[o..o + n]).unwrap();
                }
                assert_eq!(whole, pieces);
                assert_ne!(whole, plain);
                e.apply(0, hash, &mut whole).unwrap();
                assert_eq!(whole, plain);
            }
        }
    }
    #[test]
    fn prefix8_default_matches_upstream_vector_and_constants_are_parameters() {
        let mut d = [0u8; 8];
        Prefix8::DEFAULT.apply(0, 0x1234_5678, &mut d);
        assert_eq!(d, 0x2d32_f00f_4761_032du64.to_le_bytes());
        let mut other = [0u8; 8];
        Prefix8 { lo: 0, hi: 0 }.apply(0, 0x1234_5678, &mut other);
        assert_ne!(d, other);
        let mut tail = [0u8; 4];
        Prefix8::DEFAULT.apply(8, 0x1234_5678, &mut tail);
        assert_eq!(tail, [0; 4]);
    }
    #[test]
    fn invalid_orders_are_rejected() {
        let mut p = profile(GeneratorKind::Classic, None);
        p.odd_branch_order = [0, 0, 1, 2, 3, 4];
        assert!(CxdecEngine::new(p).is_err());
    }
}
