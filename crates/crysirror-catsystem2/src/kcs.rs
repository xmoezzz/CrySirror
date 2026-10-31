// SPDX-License-Identifier: MPL-2.0
//! KCS v2 stack machine. Numeric addresses are offsets in VM-owned bytes;
//! they are never native pointers. Semantics were checked statically against
//! the owner-supplied cs2.exe; see SOURCE.json for the evidence boundary.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Read;

pub const MAX_SCRIPT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_MEMORY_BYTES: usize = 32 * 1024 * 1024;
#[derive(Clone, Debug)]
pub struct Program {
    pub code: Vec<u8>,
    pub globals_bytes: u32,
    pub stack_bytes: u32,
    pub strings: Vec<(u32, Vec<u8>)>,
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .context("truncated KCS dword")?
            .try_into()?,
    ))
}
impl Program {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() >= 52 && bytes.len() <= MAX_SCRIPT_BYTES,
            "KCS source byte budget"
        );
        ensure!(&bytes[..4] == b"KCS\0", "missing KCS signature");
        let h = (0..13)
            .map(|i| u32_at(bytes, i * 4))
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            h[1] == 2 && h[2] == 0 && h[3] == 52 && h[11] == 0,
            "unsupported KCS header/version"
        );
        ensure!(
            h[5] == 52u32.checked_add(h[4]).context("KCS code overflow")?,
            "KCS table offset"
        );
        ensure!(
            h[10] == h[5].checked_add(h[7]).context("KCS table overflow")?,
            "KCS logical extent"
        );
        let expected = usize::try_from(h[10])?
            .checked_sub(52)
            .context("KCS inflated extent")?;
        ensure!(
            expected <= MAX_SCRIPT_BYTES
                && h[12] as usize <= 2 * MAX_SCRIPT_BYTES
                && h[12] as usize >= expected,
            "KCS inflated budget"
        );
        ensure!(
            h[6] <= 65536 && u64::from(h[6]) * 8 <= u64::from(h[7]),
            "KCS string index budget"
        );
        ensure!(
            h[4] > 0
                && h[8] as usize <= MAX_MEMORY_BYTES
                && h[9] >= 16
                && u64::from(h[8]) + u64::from(h[9]) <= MAX_MEMORY_BYTES as u64,
            "KCS memory budget"
        );
        let mut decoder = flate2::read::ZlibDecoder::new(&bytes[52..]);
        let mut body = Vec::new();
        decoder
            .by_ref()
            .take(expected as u64 + 1)
            .read_to_end(&mut body)?;
        ensure!(
            body.len() == expected && decoder.total_in() as usize == bytes.len() - 52,
            "KCS zlib length/trailing bytes"
        );
        let table = body.get(h[4] as usize..).context("KCS table bounds")?;
        let mut strings = Vec::new();
        let mut decoded_string_bytes = 0usize;
        for index in 0..h[6] as usize {
            let offset = u32_at(table, index * 8)? as usize;
            let address = u32_at(table, index * 8 + 4)?;
            ensure!(offset >= h[6] as usize * 8, "KCS string overlaps index");
            let rest = table.get(offset..).context("KCS string offset")?;
            let end = rest
                .iter()
                .position(|b| *b == 0)
                .context("KCS unterminated static string")?;
            decoded_string_bytes = decoded_string_bytes
                .checked_add(end + 1)
                .context("KCS static string byte overflow")?;
            ensure!(
                decoded_string_bytes <= MAX_SCRIPT_BYTES,
                "KCS cumulative static string budget"
            );
            ensure!(
                u64::from(address) + end as u64 + 1 <= u64::from(h[8]),
                "KCS string outside globals"
            );
            strings.push((address, rest[..=end].to_vec()));
        }
        Ok(Self {
            code: body[..h[4] as usize].to_vec(),
            globals_bytes: h[8],
            stack_bytes: h[9],
            strings,
        })
    }
}

/// One unacknowledged native call. The VM commits arguments/PC only after the
/// corresponding shared service succeeds. A failure retains this exact call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeCall {
    pub pc: u32,
    pub id: u32,
    pub arguments: Vec<u32>,
    next: u32,
    stack_after_arguments: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Native(NativeCall),
    BudgetExhausted,
    Halted,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Machine {
    pub pc: u32,
    pub stack: u32,
    pub frame_end: u32,
    pub frame_base: u32,
    pub accumulator: u32,
    pub memory: Vec<u8>,
    pub heap: Vec<u8>,
    pub instructions: u64,
    pub halted: bool,
    pub pending: Option<NativeCall>,
    #[serde(skip)]
    undo: Option<Vec<(u32, Vec<u8>)>>,
}
#[derive(Clone, Copy)]
struct Operand {
    flags: u8,
    value: u32,
}
impl Machine {
    pub fn new(program: &Program, heap_bytes: u32) -> Result<Self> {
        ensure!(
            (4..=16 * 1024 * 1024).contains(&heap_bytes),
            "KCS heap budget"
        );
        ensure!(
            !program.code.is_empty() && program.code.len() <= MAX_SCRIPT_BYTES,
            "KCS code byte budget"
        );
        let memory_bytes = u64::from(program.globals_bytes) + u64::from(program.stack_bytes);
        ensure!(
            memory_bytes <= MAX_MEMORY_BYTES as u64 && program.stack_bytes >= 16,
            "KCS memory budget"
        );
        ensure!(
            program.strings.len() <= 65536,
            "KCS static string count budget"
        );
        let mut copied = 0usize;
        for (address, bytes) in &program.strings {
            copied = copied
                .checked_add(bytes.len())
                .context("KCS string byte overflow")?;
            ensure!(
                copied <= MAX_SCRIPT_BYTES
                    && u64::from(*address) + bytes.len() as u64 <= u64::from(program.globals_bytes),
                "KCS static strings outside global memory/budget"
            );
        }
        let mut memory = vec![0; memory_bytes as usize];
        for (address, bytes) in &program.strings {
            memory[*address as usize..*address as usize + bytes.len()].copy_from_slice(bytes);
        }
        Ok(Self {
            pc: 0,
            stack: program.globals_bytes,
            frame_end: program.globals_bytes,
            frame_base: program.globals_bytes,
            accumulator: 0,
            memory,
            heap: vec![0; heap_bytes as usize],
            instructions: 0,
            halted: false,
            pending: None,
            undo: None,
        })
    }
    fn region(&self, address: u32, count: usize) -> Result<&[u8]> {
        let (bytes, at) = if address & 0x80000000 != 0 {
            (&self.heap, (address & 0x7fffffff) as usize)
        } else {
            (&self.memory, address as usize)
        };
        let end = at
            .checked_add(count)
            .context("KCS memory extent overflow")?;
        bytes
            .get(at..end)
            .context("KCS address outside owned memory")
    }
    pub fn read_word(&self, address: u32) -> Result<u32> {
        u32_at(self.region(address, 4)?, 0)
    }
    /// Read one native byte string without exposing a host pointer.
    pub fn read_cstring(&self, address: u32, limit: usize) -> Result<Vec<u8>> {
        ensure!(limit <= 65536, "KCS byte string limit");
        let mut value = Vec::new();
        for i in 0..=limit {
            let at = address
                .checked_add(i as u32)
                .context("KCS string address overflow")?;
            let byte = self.region(at, 1)?[0];
            if byte == 0 {
                return Ok(value);
            }
            ensure!(i < limit, "KCS byte string budget");
            value.push(byte);
        }
        unreachable!()
    }
    pub fn write_bytes(&mut self, address: u32, value: &[u8]) -> Result<()> {
        if self.undo.is_some() {
            let old = self.region(address, value.len())?.to_vec();
            self.undo.as_mut().unwrap().push((address, old));
        }
        let (bytes, at) = if address & 0x80000000 != 0 {
            (&mut self.heap, (address & 0x7fffffff) as usize)
        } else {
            (&mut self.memory, address as usize)
        };
        let end = at
            .checked_add(value.len())
            .context("KCS memory extent overflow")?;
        bytes
            .get_mut(at..end)
            .context("KCS write outside owned memory")?
            .copy_from_slice(value);
        Ok(())
    }
    fn push(&mut self, value: u32) -> Result<()> {
        self.write_bytes(self.stack, &value.to_le_bytes())?;
        self.stack = self.stack.checked_add(4).context("KCS stack overflow")?;
        Ok(())
    }
    fn pop(&mut self) -> Result<u32> {
        self.stack = self.stack.checked_sub(4).context("KCS stack underflow")?;
        self.read_word(self.stack)
    }
    fn raw_operand(&mut self, code: &[u8]) -> Result<Operand> {
        let at = self.pc as usize;
        let flags = *code.get(at).context("truncated KCS operand type")?;
        let mut value = u32_at(code, at + 1)?;
        self.pc = self.pc.checked_add(5).context("KCS PC overflow")?;
        if flags & 0x40 != 0 {
            value = self.pop()?;
        }
        Ok(Operand { flags, value })
    }
    fn operand(&mut self, code: &[u8]) -> Result<Operand> {
        let mut op = self.raw_operand(code)?;
        if op.flags & 0x10 != 0 {
            op.value = match (op.flags as i8 >= 0, op.flags & 15, op.flags & 0x20 != 0) {
                (true, 1, false) => self.region(op.value, 1)?[0] as u32,
                (true, 1, true) => self.region(op.value, 1)?[0] as i8 as i32 as u32,
                (true, 2, false) => {
                    u16::from_le_bytes(self.region(op.value, 2)?.try_into()?) as u32
                }
                (true, 2, true) => {
                    i16::from_le_bytes(self.region(op.value, 2)?.try_into()?) as i32 as u32
                }
                _ => self.read_word(op.value)?,
            };
        }
        Ok(op)
    }
    fn store(&mut self, op: Operand, value: u32) -> Result<()> {
        let count = if op.flags as i8 >= 0 {
            match op.flags & 15 {
                1 => 1,
                2 => 2,
                _ => 4,
            }
        } else {
            4
        };
        self.write_bytes(op.value, &value.to_le_bytes()[..count])
    }
    fn jump(&mut self, code: &[u8], target: u32) -> Result<()> {
        ensure!(
            (target as usize) < code.len(),
            "KCS jump target outside code: {target:#x}"
        );
        self.pc = target;
        Ok(())
    }
    /// 8762A0/877310 resume an original native71 yield. A normal owner tick
    /// pushes0 without touching its event buffer; a message copies16bytes then
    /// pushes1. Validate every write before mutation, preserving source stack.
    pub fn resume_native_event(&mut self,event:Option<(u32,[u32;4])>)->Result<()> {
        ensure!(self.pending.is_none()&&!self.halted,"KCS cannot resume native event at pending call/halt");
        self.region(self.stack,4)?;self.stack.checked_add(4).context("KCS event resume stack overflow")?;
        if let Some((address,_))=event{self.region(address,16)?;}
        if let Some((address,words))=event{let mut bytes=[0u8;16];for (out,word) in bytes.chunks_exact_mut(4).zip(words){out.copy_from_slice(&word.to_le_bytes());}self.write_bytes(address,&bytes)?;}
        self.push(u32::from(event.is_some()))?;Ok(())
    }
    pub fn acknowledge(&mut self, result: Option<u32>) -> Result<()> {
        self.commit_native(result, &[])
    }
    /// Validate every native out parameter and result slot before committing
    /// any memory change. Overlapping writes retain native declaration order.
    pub fn commit_native(&mut self, result: Option<u32>, writes: &[(u32, Vec<u8>)]) -> Result<()> {
        self.preflight_native(result.is_some(), writes)?;
        for (address, bytes) in writes {
            self.write_bytes(*address, bytes)?;
        }
        let call = self
            .pending
            .take()
            .context("KCS has no pending native call")?;
        self.pc = call.next;
        self.stack = call.stack_after_arguments;
        if let Some(value) = result {
            self.push(value)?;
        }
        Ok(())
    }
    /// Admission check before publishing a native resource or shared operation.
    pub fn preflight_native(&self, result: bool, writes: &[(u32, Vec<u8>)]) -> Result<()> {
        let pending = self
            .pending
            .as_ref()
            .context("KCS has no pending native call")?;
        ensure!(writes.len() <= 64, "KCS native write count budget");
        let mut total = 0usize;
        for (address, bytes) in writes {
            total = total
                .checked_add(bytes.len())
                .context("KCS native write byte overflow")?;
            ensure!(total <= MAX_SCRIPT_BYTES, "KCS native write byte budget");
            self.region(*address, bytes.len())?;
        }
        if result {
            self.region(pending.stack_after_arguments, 4)?;
        }
        Ok(())
    }
    pub fn step(&mut self, program: &Program, budget: u32) -> Result<Event> {
        if let Some(call) = &self.pending {
            return Ok(Event::Native(call.clone()));
        }
        if self.halted {
            return Ok(Event::Halted);
        }
        for _ in 0..budget {
            let before = (
                self.pc,
                self.stack,
                self.frame_end,
                self.frame_base,
                self.accumulator,
                self.instructions,
                self.halted,
            );
            self.undo = Some(Vec::new());
            match self.instruction(program) {
                Ok(Some(event)) => {
                    self.undo = None;
                    return Ok(event);
                }
                Ok(None) => self.undo = None,
                Err(e) => {
                    let undo = self.undo.take().unwrap();
                    for (address, bytes) in undo.into_iter().rev() {
                        self.write_bytes(address, &bytes)?;
                    }
                    (
                        self.pc,
                        self.stack,
                        self.frame_end,
                        self.frame_base,
                        self.accumulator,
                        self.instructions,
                        self.halted,
                    ) = before;
                    return Err(e);
                }
            }
        }
        Ok(Event::BudgetExhausted)
    }
    fn instruction(&mut self, program: &Program) -> Result<Option<Event>> {
        let code = &program.code;
        let original = self.pc;
        let op = *code.get(self.pc as usize).context("KCS PC outside code")?;
        self.pc += 1;
        self.instructions += 1;
        match op {
            0 => {}
            1..=0x14 => {
                let a = self.operand(code)?;
                let b = if matches!(op, 8 | 0xc) {
                    Operand { flags: 0, value: 0 }
                } else {
                    self.operand(code)?
                };
                let (x, y) = (a.value, b.value);
                let (fx, fy) = (f32::from_bits(x), f32::from_bits(y));
                let float = if op <= 4 {
                    a.flags & 4 != 0 || b.flags & 4 != 0
                } else {
                    a.flags & 4 != 0
                };
                let signed = a.flags & 0x20 != 0 || b.flags & 0x20 != 0;
                let result = match op {
                    1 if float => (fx + fy).to_bits(),
                    1 => x.wrapping_add(y),
                    2 if float => (fx - fy).to_bits(),
                    2 => x.wrapping_sub(y),
                    3 if float => (fx * fy).to_bits(),
                    3 => x.wrapping_mul(y),
                    4 if float => {
                        if fy == 0.0 {
                            0
                        } else {
                            (fx / fy).to_bits()
                        }
                    }
                    4 if signed => {
                        if y == 0 {
                            0
                        } else {
                            (x as i32)
                                .checked_div(y as i32)
                                .context("native signed division overflow")?
                                as u32
                        }
                    }
                    4 => {
                        if y == 0 {
                            0
                        } else {
                            x / y
                        }
                    }
                    5 => {
                        if y == 0 {
                            0
                        } else if signed {
                            (x as i32)
                                .checked_rem(y as i32)
                                .context("native signed remainder overflow")?
                                as u32
                        } else {
                            x % y
                        }
                    }
                    6 => x & y,
                    7 => x | y,
                    8 => !x,
                    9 => x ^ y,
                    0xa => x.wrapping_shl(y & 31),
                    0xb => x.wrapping_shr(y & 31),
                    0xc => (x == 0) as u32,
                    0xd => (x != 0 && y != 0) as u32,
                    0xe => (x != 0 || y != 0) as u32,
                    0xf => {
                        if float {
                            (fx > fy) as u32
                        } else if signed {
                            ((x as i32) > (y as i32)) as u32
                        } else {
                            (x > y) as u32
                        }
                    }
                    0x10 => {
                        if float {
                            (fx < fy) as u32
                        } else if signed {
                            ((x as i32) < (y as i32)) as u32
                        } else {
                            (x < y) as u32
                        }
                    }
                    0x11 => {
                        if float {
                            (fx >= fy) as u32
                        } else if signed {
                            ((x as i32) >= (y as i32)) as u32
                        } else {
                            (x >= y) as u32
                        }
                    }
                    0x12 => {
                        if float {
                            (fx <= fy) as u32
                        } else if signed {
                            ((x as i32) <= (y as i32)) as u32
                        } else {
                            (x <= y) as u32
                        }
                    }
                    0x13 => {
                        if float {
                            (fx == fy) as u32
                        } else {
                            (x == y) as u32
                        }
                    }
                    0x14 => {
                        if float {
                            (fx != fy) as u32
                        } else {
                            (x != y) as u32
                        }
                    }
                    _ => unreachable!(),
                };
                self.push(result)?;
            }
            0x15 => {
                let dest = self.raw_operand(code)?;
                let value = self.operand(code)?.value;
                self.store(dest, value)?;
                let result = if dest.flags as i8 >= 0 {
                    match dest.flags & 15 {
                        1 => value & 255,
                        2 => value & 65535,
                        _ => value,
                    }
                } else {
                    value
                };
                self.push(result)?;
            }
            0x16..=0x19 => {
                let dest = self.raw_operand(code)?;
                // Native increment handlers always fetch/push the complete DWORD,
                // even when only its low byte/word is changed.
                let old = self.read_word(dest.value)?;
                let kind = dest.flags & 15;
                ensure!((1..=4).contains(&kind), "unsupported KCS increment width");
                let increment = op == 0x16 || op == 0x18;
                let value = if kind == 4 {
                    (f32::from_bits(old) + if increment { 1.0 } else { -1.0 }).to_bits()
                } else if increment {
                    old.wrapping_add(1)
                } else {
                    old.wrapping_sub(1)
                };
                let width = match kind {
                    1 => 1,
                    2 => 2,
                    _ => 4,
                };
                self.write_bytes(dest.value, &value.to_le_bytes()[..width])?;
                self.push(if op >= 0x18 {
                    old
                } else {
                    self.read_word(dest.value)?
                })?;
            }
            0x1a => {
                let a = self.operand(code)?;
                self.push(if a.flags & 4 != 0 {
                    (-f32::from_bits(a.value)).to_bits()
                } else {
                    a.value.wrapping_neg()
                })?;
            }
            0x1b..=0x24 => {
                let dest = self.raw_operand(code)?;
                let right = self.operand(code)?;
                let (x, y) = (self.read_word(dest.value)?, right.value);
                let float = dest.flags & 4 != 0 || right.flags & 4 != 0;
                let signed = dest.flags & 0x20 != 0 || right.flags & 0x20 != 0;
                let result = match op {
                    0x1b if y == 0 => 0,
                    0x1b if signed => (x as i32)
                        .checked_rem(y as i32)
                        .context("native signed remainder overflow")?
                        as u32,
                    0x1b => x % y,
                    0x1c => x & y,
                    0x1d if float => (f32::from_bits(x) * f32::from_bits(y)).to_bits(),
                    0x1d => x.wrapping_mul(y),
                    0x1e if float => (f32::from_bits(x) + f32::from_bits(y)).to_bits(),
                    0x1e => x.wrapping_add(y),
                    0x1f if float => (f32::from_bits(x) - f32::from_bits(y)).to_bits(),
                    0x1f => x.wrapping_sub(y),
                    0x20 if float => {
                        if f32::from_bits(y) == 0.0 {
                            y
                        } else {
                            (f32::from_bits(x) / f32::from_bits(y)).to_bits()
                        }
                    }
                    // The reviewed native unsigned compound divide stores EDX
                    // (remainder), unlike its signed IDIV branch storing EAX.
                    0x20 if y == 0 => 0,
                    0x20 if signed => (x as i32)
                        .checked_div(y as i32)
                        .context("native signed division overflow")?
                        as u32,
                    0x20 => x % y,
                    0x21 => x.wrapping_shl(y & 31),
                    0x22 if signed => ((x as i32) >> (y & 31)) as u32,
                    0x22 => x >> (y & 31),
                    0x23 => x ^ y,
                    0x24 => x | y,
                    _ => unreachable!(),
                };
                self.store(dest, result)?;
                // Compound operations return the full result, before narrow store.
                self.push(result)?;
            }
            0x25 => {
                let at = self.pc as usize;
                let flags = *code.get(at).context("truncated KCS push operand")?;
                let value = u32_at(code, at + 1)?;
                self.pc = self.pc.checked_add(5).context("KCS PC overflow")?;
                // A stack value already in place is left untouched. Only a
                // stack-supplied address is popped before dereferencing.
                let a = Operand {
                    flags,
                    value: if flags & 0x50 == 0x50 {
                        self.pop()?
                    } else {
                        value
                    },
                };
                if a.flags & 0x10 != 0 {
                    let value = self.read_word(a.value)?;
                    self.push(value)?;
                } else if a.flags & 0x40 == 0 {
                    self.push(a.value)?;
                }
            }
            0x26 => {
                let dest = self.raw_operand(code)?;
                let value = self.pop()?;
                self.store(dest, value)?;
            }
            0x27 | 0x28 => {
                let target = self.operand(code)?.value;
                let cond = self.pop()?;
                if (op == 0x27 && cond == 0) || (op == 0x28 && cond != 0) {
                    self.jump(code, target)?;
                }
                self.stack = self.frame_end;
            }
            0x29 => {
                let target = self.operand(code)?.value;
                self.jump(code, target)?;
                self.stack = self.frame_end;
            }
            0x2a => {
                let target = self.operand(code)?.value;
                let args = self.operand(code)?.value;
                self.push(self.pc)?;
                self.push(self.frame_base)?;
                self.push(self.frame_end)?;
                self.frame_base = self
                    .stack
                    .checked_sub(args)
                    .and_then(|n| n.checked_sub(12))
                    .context("KCS call argument underflow")?;
                self.frame_end = self.stack;
                self.jump(code, target)?;
            }
            0x2b => {
                let id = self.operand(code)?.value;
                let bytes = self.operand(code)?.value;
                ensure!(
                    bytes <= 4096 && bytes % 4 == 0,
                    "KCS native argument budget/alignment"
                );
                let at = self
                    .stack
                    .checked_sub(bytes)
                    .context("KCS native argument underflow")?;
                let args = (0..bytes / 4)
                    .map(|i| self.read_word(at + i * 4))
                    .collect::<Result<Vec<_>>>()?;
                let call = NativeCall {
                    pc: original,
                    id,
                    arguments: args,
                    next: self.pc,
                    stack_after_arguments: at,
                };
                self.pc = original;
                self.pending = Some(call.clone());
                return Ok(Some(Event::Native(call)));
            }
            0x2c => {
                let bytes = self.operand(code)?.value;
                let args = self.operand(code)?.value;
                ensure!(bytes <= 4096, "KCS return byte budget");
                let result = self
                    .region(
                        self.stack
                            .checked_sub(bytes)
                            .context("KCS result underflow")?,
                        bytes as usize,
                    )?
                    .to_vec();
                self.stack = self
                    .frame_base
                    .checked_add(args)
                    .and_then(|v| v.checked_add(12))
                    .context("KCS frame overflow")?;
                self.frame_end = self.pop()?;
                self.frame_base = self.pop()?;
                let target = self.pop()?;
                self.stack = self
                    .stack
                    .checked_sub(args)
                    .context("KCS return argument underflow")?;
                self.write_bytes(self.stack, &result)?;
                self.stack += bytes;
                self.jump(code, target)?;
            }
            0x2d => {
                self.pop()?;
            }
            0x2e => {
                let value = self.raw_operand(code)?.value;
                self.push(value)?;
            }
            0x2f => {
                let address = self.raw_operand(code)?.value;
                let value = self.read_word(address)?;
                self.push(value)?;
            }
            0x30 => {
                let dest = self.raw_operand(code)?.value;
                let count = self.operand(code)?.value;
                ensure!(count <= MAX_SCRIPT_BYTES as u32, "KCS copy budget");
                let source = self
                    .region(
                        self.stack
                            .checked_sub(count)
                            .context("KCS copy stack underflow")?,
                        count as usize,
                    )?
                    .to_vec();
                self.write_bytes(dest, &source)?;
            }
            0x31 => {
                let bytes = self.operand(code)?.value;
                self.frame_end = self
                    .frame_end
                    .checked_add(bytes)
                    .context("KCS frame allocation overflow")?;
                ensure!(
                    self.frame_end as usize <= self.memory.len(),
                    "KCS frame exceeds memory"
                );
                self.stack = self.frame_end;
            }
            0x32 => {
                let bytes = self.operand(code)?.value;
                self.frame_end = self
                    .frame_end
                    .checked_sub(bytes)
                    .context("KCS frame allocation underflow")?;
                self.stack = self.frame_end;
            }
            0x33 => {
                self.accumulator = self.pop()?;
            }
            0x34 => {
                let value = self.operand(code)?.value;
                let target = self.operand(code)?.value;
                if value == self.accumulator {
                    self.jump(code, target)?;
                }
            }
            0x35 => {
                let a = self.operand(code)?.value;
                let b = self.operand(code)?.value;
                let target = self.operand(code)?.value;
                if (a.min(b)..=a.max(b)).contains(&self.accumulator) {
                    self.jump(code, target)?;
                }
            }
            0x36..=0x39 => {
                let value = self.operand(code)?.value;
                let converted = match op {
                    0x36 => (value as f32).to_bits(),
                    0x37 => (value as i32 as f32).to_bits(),
                    0x38 => f32::from_bits(value) as u32,
                    _ => f32::from_bits(value) as i32 as u32,
                };
                self.push(converted)?;
            }
            0x3a => {
                let value = self.pop()?;
                self.push(value)?;
                self.push(value)?;
            }
            0x3b => {
                self.pc = original;
                self.halted = true;
                return Ok(Some(Event::Halted));
            }
            0x3c => self.stack = self.frame_end,
            0x3d => {
                let offset = self.operand(code)?.value;
                self.push(self.frame_base.wrapping_add(offset))?;
            }
            0x3e => {
                let a = self.raw_operand(code)?;
                let a = if a.flags & 0x10 != 0 {
                    self.read_word(a.value)?
                } else {
                    a.value
                };
                let b = self.operand(code)?.value;
                self.push(a.wrapping_add(b))?;
            }
            _ => bail!("unsupported KCS opcode {op:#04x} at native PC {original:#x}"),
        }
        Ok(None)
    }
}
