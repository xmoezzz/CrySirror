// Copyright (c) 2026 CrySirror contributors.
// Opcode/format definitions derived from MajiroTools (C) 2021 Mario Welzig,
// Robert Jordan. MIT; see LICENSE and SOURCE.json.
//! Bounded original MJO instruction resources. Strings retain native bytes.
use anyhow::{bail, ensure, Context, Result};
use std::collections::{BTreeMap, BTreeSet};
pub const MAX_MODULE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_INSTRUCTIONS: usize = 1_000_000;
#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    Integer(i32),
    Float(f32),
    String(Vec<u8>),
    Types(Vec<u8>),
    Variable { flags: u16, hash: u32, offset: i16 },
    Call { hash: u32, arguments: u16 },
    Jump(u32),
    Switch(Vec<u32>),
    Line(u16),
    None,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Instruction {
    pub pc: u32,
    pub next: u32,
    pub opcode: u16,
    pub operand: Operand,
}
#[derive(Clone, Debug)]
pub struct Program {
    pub entry: u32,
    pub read_mark_lines: u32,
    pub functions: BTreeMap<u32, u32>,
    pub instructions: BTreeMap<u32, Instruction>,
    pub code_size: u32,
}
struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let e = self.p.checked_add(n).context("MJO offset overflow")?;
        let b = self.b.get(self.p..e).context("truncated MJO operand")?;
        self.p = e;
        Ok(b)
    }
    fn word(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn short(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
}
/// Standard CRC-32 name hash over original encoded bytes, including caller's
/// native `$`/group suffix. No game-specific key or executable is involved.
pub fn name_hash(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb88320 } else { 0 };
        }
    }
    !crc
}
/// MJO's fixed 1024-byte CRC-table XOR. Offsets refer to the bytecode body.
pub fn xor_bytecode(bytes: &mut [u8]) {
    let mut key = [0u8; 1024];
    for i in 0..256 {
        let mut n = i as u32;
        for _ in 0..8 {
            n = (n >> 1) ^ if n & 1 != 0 { 0xedb88320 } else { 0 };
        }
        key[i * 4..i * 4 + 4].copy_from_slice(&n.to_le_bytes());
    }
    for (i, b) in bytes.iter_mut().enumerate() {
        *b ^= key[i & 1023];
    }
}
fn encoding(op: u16) -> Option<&'static str> {
    match op {
        0x800 => Some("i"),
        0x801 | 0x840 | 0x842 => Some("s"),
        0x802 | 0x837 => Some("v"),
        0x803 => Some("r"),
        0x80f | 0x810 => Some("C"),
        0x834 | 0x835 => Some("c"),
        0x829 | 0x836 => Some("t"),
        0x83a => Some("l"),
        0x850 => Some("S"),
        0x82c..=0x82e | 0x830..=0x833 | 0x838..=0x83d | 0x843 | 0x845 | 0x847 => Some("j"),
        0x82b | 0x82f | 0x83e | 0x83f | 0x841 | 0x844 | 0x846 | 0x190 | 0x191 | 0x198 | 0x1a0
        | 0x1a1 | 0x1a8 | 0x1a9 => Some(""),
        _ => {
            let group = op & !7;
            let ty = op & 7;
            let types = match group {
                0x100 | 0x108 | 0x120 => 2,
                0x110 | 0x128 | 0x130 | 0x168 | 0x170 | 0x178 | 0x180 | 0x188 => 1,
                0x118 | 0x138 | 0x140 | 0x148 | 0x150 => 3,
                0x158 | 0x160 => 6,
                _ => 0,
            };
            if types > 0 && ty < types {
                return Some("");
            }
            let (base, array) = if (0x1b0..=0x205).contains(&op) {
                (0x1b0, false)
            } else if (0x210..=0x265).contains(&op) {
                (0x210, false)
            } else if (0x270..=0x2c2).contains(&op) {
                (0x270, true)
            } else if (0x2d0..=0x322).contains(&op) {
                (0x2d0, true)
            } else {
                return None;
            };
            let index = (op - base) / 8;
            let ty = (op - base) % 8;
            let allowed = match index {
                0 => {
                    if array {
                        3
                    } else {
                        6
                    }
                }
                1 | 2 | 5 => 2,
                3 | 6..=10 => 1,
                4 => 3,
                _ => 0,
            };
            (ty < allowed).then_some("v")
        }
    }
}
fn jump(base: usize, relative: i32) -> Result<u32> {
    u32::try_from(base as i64 + relative as i64).context("MJO jump underflow/overflow")
}
impl Program {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_MODULE_BYTES, "MJO module byte budget");
        let mut r = Reader { b: bytes, p: 0 };
        let signature = r.take(16)?;
        let encrypted = match signature {
            b"MajiroObjX1.000\0" => true,
            b"MajiroObjV1.000\0" => false,
            _ => bail!("unsupported MJO signature/version"),
        };
        let entry = r.word()?;
        let read_mark_lines = r.word()?;
        ensure!(
            read_mark_lines <= MAX_INSTRUCTIONS as u32,
            "MJO read-mark budget"
        );
        let count = r.word()? as usize;
        ensure!(count <= 65536, "MJO function count budget");
        let mut functions = BTreeMap::new();
        for _ in 0..count {
            let hash = r.word()?;
            let offset = r.word()?;
            ensure!(
                functions.insert(hash, offset).is_none(),
                "duplicate MJO function hash"
            );
        }
        let code_size = r.word()?;
        ensure!(
            code_size > 0 && code_size as usize == bytes.len() - r.p,
            "MJO bytecode extent/trailing data"
        );
        let mut code = r.take(code_size as usize)?.to_vec();
        if encrypted {
            xor_bytecode(&mut code);
        }
        let mut r = Reader { b: &code, p: 0 };
        let mut instructions = BTreeMap::new();
        let mut targets = BTreeSet::from([entry]);
        targets.extend(functions.values().copied());
        while r.p < code.len() {
            ensure!(
                instructions.len() < MAX_INSTRUCTIONS,
                "MJO instruction budget"
            );
            let pc = r.p as u32;
            let opcode = r.short()?;
            let fmt = encoding(opcode)
                .with_context(|| format!("unknown MJO opcode {opcode:#x} at {pc:#x}"))?;
            let operand = match fmt {
                "i" => Operand::Integer(r.word()? as i32),
                "r" => Operand::Float(f32::from_bits(r.word()?)),
                "s" => {
                    let n = r.short()? as usize;
                    ensure!(n > 0, "MJO zero-length string unsupported");
                    let b = r.take(n)?;
                    ensure!(b[n - 1] == 0, "MJO string terminator");
                    Operand::String(b[..n - 1].to_vec())
                }
                "t" => {
                    let n = r.short()? as usize;
                    let t = r.take(n)?.to_vec();
                    ensure!(t.iter().all(|x| *x <= 5), "MJO type list");
                    Operand::Types(t)
                }
                "v" => Operand::Variable {
                    flags: r.short()?,
                    hash: r.word()?,
                    offset: r.short()? as i16,
                },
                "c" | "C" => {
                    let hash = r.word()?;
                    if fmt == "C" {
                        ensure!(
                            r.word()? == 0,
                            "MJO cached code address is not a file operand"
                        );
                    }
                    Operand::Call {
                        hash,
                        arguments: r.short()?,
                    }
                }
                "j" => {
                    let rel = r.word()? as i32;
                    let target = jump(r.p, rel)?;
                    targets.insert(target);
                    Operand::Jump(target)
                }
                "l" => Operand::Line(r.short()?),
                "S" => {
                    let n = r.short()? as usize;
                    let mut out = Vec::with_capacity(n);
                    for _ in 0..n {
                        let rel = r.word()? as i32;
                        let target = jump(r.p, rel)?;
                        targets.insert(target);
                        out.push(target);
                    }
                    Operand::Switch(out)
                }
                _ => Operand::None,
            };
            instructions.insert(
                pc,
                Instruction {
                    pc,
                    next: r.p as u32,
                    opcode,
                    operand,
                },
            );
        }
        ensure!(
            targets.iter().all(|t| instructions.contains_key(t)),
            "MJO target is not an instruction boundary"
        );
        Ok(Self {
            entry,
            read_mark_lines,
            functions,
            instructions,
            code_size,
        })
    }
    pub fn instruction(&self, pc: u32) -> Result<&Instruction> {
        self.instructions
            .get(&pc)
            .context("MJO PC is not an instruction boundary")
    }
}
