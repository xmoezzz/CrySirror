//! Native BGI BP bytecode and native ABI state. No renderer, pixels or mixer.
//! Supported bounded profile and source provenance are recorded in SOURCE.json.
#![forbid(unsafe_code)]
use anyhow::{Context, Result, bail, ensure};
pub mod native_abi;
pub mod vm_opcode;
const CODE: u32 = 0x0400_0000;
const FRAME: u32 = 0x0800_0000;
const MASK: u32 = 0x01ff_ffff;
const RING: usize = 4096;
const MEMORY: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeCall {
    pub group: u8,
    pub id: u16,
    pub args: Vec<i32>,
    pub outputs: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Continue,
    Halted,
    Call(NativeCall),
}
#[derive(Clone)]
struct Pending {
    call: NativeCall,
    next: usize,
}
/// Independent BP owner. Native calls remain pending until the host operation
/// succeeds; an unsupported call cannot become a zero-returning success.
pub struct Machine {
    code: Vec<u8>,
    pc: usize,
    ring: Vec<i32>,
    sp: usize,
    memory: Vec<u8>,
    base: u32,
    returns: Vec<usize>,
    pending: Option<Pending>,
    halted: bool,
}
impl Machine {
    pub fn from_bp(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() >= 8 && bytes.len() <= 16 * 1024 * 1024,
            "BGI BP byte budget/header"
        );
        let header = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        let len = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        ensure!(
            header >= 8
                && header <= bytes.len()
                && header.checked_add(len) == Some(bytes.len())
                && len > 0,
            "inconsistent native BP image"
        );
        Ok(Self {
            code: bytes[header..].to_vec(),
            pc: 0,
            ring: vec![0; RING],
            sp: 0,
            memory: vec![0; MEMORY],
            base: 0,
            returns: vec![],
            pending: None,
            halted: false,
        })
    }
    pub fn pc(&self) -> usize {
        self.pc
    }
    pub fn halted(&self) -> bool {
        self.halted
    }
    pub fn stack_index(&self) -> usize {
        self.sp
    }
    pub fn top(&self) -> i32 {
        self.ring[(self.sp + RING - 1) % RING]
    }
    fn push(&mut self, v: i32) {
        self.ring[self.sp] = v;
        self.sp = (self.sp + 1) % RING;
    }
    fn pop(&mut self) -> i32 {
        self.sp = (self.sp + RING - 1) % RING;
        self.ring[self.sp]
    }
    fn byte(&self, at: &mut usize) -> Result<u8> {
        let v = *self.code.get(*at).context("truncated BP instruction")?;
        *at += 1;
        Ok(v)
    }
    fn word(&self, at: &mut usize) -> Result<i16> {
        let a = self.byte(at)?;
        let b = self.byte(at)?;
        Ok(i16::from_le_bytes([a, b]))
    }
    fn dword(&self, at: &mut usize) -> Result<i32> {
        let a = self.word(at)? as u16;
        let b = self.word(at)? as u16;
        Ok((u32::from(a) | (u32::from(b) << 16)) as i32)
    }
    fn range(&self, ptr: u32, count: usize) -> Result<&[u8]> {
        let at = (ptr & MASK) as usize;
        let data = match ptr & !MASK {
            0 => &self.memory[..],
            CODE => &self.code[..],
            FRAME => &self.memory[..],
            _ => bail!("unsupported BP pointer segment {ptr:#x}"),
        };
        data.get(at..at.checked_add(count).context("BP memory overflow")?)
            .context("BP read outside memory")
    }
    fn write(&mut self, ptr: u32, data: &[u8]) -> Result<()> {
        ensure!(
            matches!(ptr & !MASK, 0 | FRAME),
            "BP write to immutable or unsupported segment"
        );
        let at = (ptr & MASK) as usize;
        self.memory
            .get_mut(at..at.checked_add(data.len()).context("BP write overflow")?)
            .context("BP write outside frame region")?
            .copy_from_slice(data);
        Ok(())
    }
    pub fn string(&self, ptr: i32) -> Result<String> {
        let ptr = ptr as u32;
        let at = (ptr & MASK) as usize;
        let data = match ptr & !MASK {
            CODE => &self.code[..],
            0 | FRAME => &self.memory[..],
            _ => bail!("BGI string segment"),
        };
        let tail = data.get(at..).context("BGI string address")?;
        let end = tail
            .iter()
            .take(64 * 1024)
            .position(|&b| b == 0)
            .context("unterminated BGI string")?;
        let (s, _, bad) = encoding_rs::SHIFT_JIS.decode(&tail[..end]);
        ensure!(!bad, "invalid BGI CP932 string");
        Ok(s.into_owned())
    }
    pub fn commit_call(&mut self, outputs: &[i32]) -> Result<()> {
        let pending = self
            .pending
            .as_ref()
            .context("no pending BGI native call")?;
        ensure!(
            outputs.len() == pending.call.outputs,
            "BGI native result count differs from ABI"
        );
        let pending = self.pending.take().unwrap();
        for _ in &pending.call.args {
            self.pop();
        }
        for &v in outputs {
            self.push(v);
        }
        self.pc = pending.next;
        Ok(())
    }
    pub fn step(&mut self) -> Result<Step> {
        if self.halted {
            return Ok(Step::Halted);
        }
        if let Some(pending) = &self.pending {
            return Ok(Step::Call(pending.call.clone()));
        }
        let start = self.pc;
        let mut at = start;
        let op = self.byte(&mut at)?;
        // Validate immediates before modifying any stack or memory state.
        let result = (|| -> Result<Step> {
            match op {
                0x00 => {
                    let n = self.byte(&mut at)? as i8;
                    self.push(i32::from(n));
                }
                0x01 => {
                    let n = self.word(&mut at)?;
                    self.push(i32::from(n));
                }
                0x02 => {
                    let n = self.dword(&mut at)?;
                    self.push(n);
                }
                0x04 => {
                    let d = i32::from(self.word(&mut at)?);
                    let ptr = i64::from(self.base) - i64::from(d);
                    ensure!(
                        (0..MEMORY as i64).contains(&ptr),
                        "BGI frame pointer displacement"
                    );
                    self.push((FRAME | ptr as u32) as i32);
                }
                0x05 | 0x06 => {
                    let d = i64::from(self.word(&mut at)?);
                    let ptr = start as i64 + d;
                    ensure!(
                        (0..self.code.len() as i64).contains(&ptr),
                        "BGI relative code pointer"
                    );
                    self.push((ptr as u32 | if op == 5 { CODE } else { 0 }) as i32);
                }
                0x08 => {
                    let width = self.byte(&mut at)?;
                    ensure!(width <= 2, "BGI load width");
                    let ptr = self.pop() as u32;
                    let b = self.range(ptr, 1 << width)?;
                    let n = match width {
                        0 => i32::from(b[0] as i8),
                        1 => i32::from(i16::from_le_bytes(b.try_into().unwrap())),
                        _ => i32::from_le_bytes(b.try_into().unwrap()),
                    };
                    self.push(n);
                }
                0x09 | 0x0a => {
                    let width = self.byte(&mut at)?;
                    ensure!(width <= 2, "BGI store width");
                    let (ptr, v) = if op == 9 {
                        let v = self.pop();
                        (self.pop() as u32, v)
                    } else {
                        let p = self.pop() as u32;
                        (p, self.pop())
                    };
                    self.write(ptr, &v.to_le_bytes()[..1 << width])?;
                    if op == 9 {
                        self.push(v);
                    }
                }
                0x10 => self.push(self.base as i32),
                0x11 => {
                    let base = self.pop() as u32;
                    ensure!(base < (MEMORY as u32), "BGI frame base exceeds profile");
                    self.base = base;
                }
                0x14 | 0x16 => {
                    let target = self.pop() as u32;
                    ensure!(
                        target > 0 && (target as usize) < self.code.len(),
                        "BGI branch target"
                    );
                    if op == 0x16 {
                        ensure!(self.returns.len() < 4096, "BGI call depth");
                        self.returns.push(at);
                    }
                    at = target as usize;
                }
                0x15 => {
                    let pred = self.byte(&mut at)?;
                    ensure!(pred <= 5, "BGI branch predicate");
                    let target = self.pop() as u32;
                    let value = self.pop();
                    let take = match pred {
                        0 => value != 0,
                        1 => value == 0,
                        2 => value > 0,
                        3 => value >= 0,
                        4 => value <= 0,
                        _ => value < 0,
                    };
                    if take {
                        ensure!(
                            target > 0 && (target as usize) < self.code.len(),
                            "BGI conditional branch target"
                        );
                        at = target as usize;
                    }
                }
                0x17 => {
                    if let Some(next) = self.returns.pop() {
                        at = next;
                    } else {
                        self.halted = true;
                        self.pc = at;
                        return Ok(Step::Halted);
                    }
                }
                0x20..=0x27 | 0x29..=0x2b | 0x30..=0x35 | 0x38 | 0x39 | 0x56 | 0x57 => {
                    let r = self.pop();
                    let l = self.pop();
                    let v = match op {
                        0x20 => l.wrapping_add(r),
                        0x21 => l.wrapping_sub(r),
                        0x22 => l.wrapping_mul(r),
                        0x23 => {
                            if r == 0 {
                                -1
                            } else {
                                l.wrapping_div(r)
                            }
                        }
                        0x24 => {
                            if r == 0 {
                                -1
                            } else {
                                l.wrapping_rem(r)
                            }
                        }
                        0x25 => l & r,
                        0x26 => l | r,
                        0x27 => l ^ r,
                        0x29 => l.wrapping_shl((r & 31) as u32),
                        0x2a => ((l as u32) >> (r & 31)) as i32,
                        0x2b => l >> (r & 31),
                        0x30 => i32::from(l == r),
                        0x31 => i32::from(l != r),
                        0x32 => i32::from(l <= r),
                        0x33 => i32::from(l >= r),
                        0x34 => i32::from(l < r),
                        0x35 => i32::from(l > r),
                        0x38 => i32::from(l != 0 && r != 0),
                        0x39 => i32::from(l != 0 || r != 0),
                        0x56 => ((i64::from(l) * i64::from(r)) >> 16) as i32,
                        _ => {
                            if r == 0 {
                                0
                            } else {
                                ((i64::from(l) << 16) / i64::from(r)) as i32
                            }
                        }
                    };
                    self.push(v);
                }
                0x28 => {
                    let v = self.pop();
                    self.push(!v);
                }
                0x3a => {
                    let v = self.pop();
                    self.push(i32::from(v == 0));
                }
                0x40 => {
                    let f = self.pop();
                    let t = self.pop();
                    let c = self.pop();
                    self.push(if c != 0 { t } else { f });
                }
                0x42 => {
                    let d = self.pop();
                    let m = self.pop();
                    let a = self.pop();
                    self.push(if d == 0 {
                        -1
                    } else {
                        (i64::from(a) * i64::from(m) / i64::from(d)) as i32
                    });
                }
                0x68 => {
                    let p = self.pop() as u32;
                    let mut n = 0;
                    while self.range(p + n, 1)?[0] != 0 {
                        n += 1;
                        ensure!(n <= 65536, "BGI string length budget");
                    }
                    self.push(n as i32);
                }
                0x80 | 0x81 | 0x90 | 0x91 | 0x92 | 0xa0 | 0xb0 | 0xc0 => {
                    let id = u16::from(self.byte(&mut at)?);
                    let abi =
                        native_abi::lookup(op, id).context("unregistered BGI native selector")?;
                    let args = (0..abi.argc)
                        .rev()
                        .map(|i| self.ring[(self.sp + RING - 1 - i) % RING])
                        .collect();
                    let call = NativeCall {
                        group: op,
                        id,
                        args,
                        outputs: abi.stack_outputs,
                    };
                    self.pending = Some(Pending {
                        call: call.clone(),
                        next: at,
                    });
                    return Ok(Step::Call(call));
                }
                _ => bail!(
                    "unsupported native BGI opcode {op:#04x} ({})",
                    vm_opcode::lookup_bp_opcode(op)
                        .map(|s| s.symbol)
                        .unwrap_or("unregistered")
                ),
            }
            self.pc = at;
            Ok(Step::Continue)
        })();
        result.with_context(|| format!("BGI BP instruction {start:#x}"))
    }
}

/// Author-owned BP fixture builder, using the native header and opcodes.
#[derive(Default)]
pub struct Builder {
    pub code: Vec<u8>,
    strings: Vec<(usize, String)>,
}
impl Builder {
    pub fn int(&mut self, value: i32) -> &mut Self {
        self.code.push(2);
        self.code.extend(value.to_le_bytes());
        self
    }
    pub fn string(&mut self, text: &str) -> &mut Self {
        let at = self.code.len();
        self.code.extend([5, 0, 0]);
        self.strings.push((at, text.into()));
        self
    }
    pub fn call(&mut self, group: u8, id: u8) -> &mut Self {
        self.code.extend([group, id]);
        self
    }
    pub fn ret(&mut self) -> &mut Self {
        self.code.push(0x17);
        self
    }
    pub fn finish(mut self) -> Vec<u8> {
        for (at, text) in self.strings {
            let displacement =
                i16::try_from(self.code.len() - at).expect("fixture string displacement");
            self.code[at + 1..at + 3].copy_from_slice(&displacement.to_le_bytes());
            let (encoded, _, bad) = encoding_rs::SHIFT_JIS.encode(&text);
            assert!(!bad, "fixture CP932 text");
            self.code.extend_from_slice(&encoded);
            self.code.push(0);
        }
        let mut image = vec![0; 16];
        image[..4].copy_from_slice(&16u32.to_le_bytes());
        image[4..8].copy_from_slice(&(self.code.len() as u32).to_le_bytes());
        image.extend(self.code);
        image
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_native_call_is_not_acknowledged_on_host_failure() {
        let mut b = Builder::default();
        b.int(7).call(0x80, 0x58).ret();
        let mut m = Machine::from_bp(&b.finish()).unwrap();
        m.step().unwrap();
        let before = m.pc();
        let Step::Call(c) = m.step().unwrap() else {
            panic!()
        };
        assert_eq!(c.args, [7]);
        assert_eq!(m.pc(), before);
        assert_eq!(m.step().unwrap(), Step::Call(c));
        assert!(m.commit_call(&[1]).is_err());
        assert_eq!(m.pc(), before);
        m.commit_call(&[]).unwrap();
        assert_eq!(m.step().unwrap(), Step::Halted);
    }
    #[test]
    fn wrapping_arithmetic_and_native_operand_ring_are_preserved() {
        let mut b = Builder::default();
        b.int(i32::MAX).int(1);
        b.code.push(0x20);
        b.ret();
        let mut m = Machine::from_bp(&b.finish()).unwrap();
        m.step().unwrap();
        m.step().unwrap();
        m.step().unwrap();
        assert_eq!(m.top(), i32::MIN);
        for n in 0..4097 {
            m.push(n);
        }
        assert_eq!(m.stack_index(), 2);
        assert_eq!(m.pop(), 4096);
        assert_eq!(m.pop(), 4095);
    }
    #[test]
    fn malformed_images_and_unknown_instructions_fail_visibly() {
        assert!(Machine::from_bp(&[0; 16]).is_err());
        let mut b = Builder::default();
        b.code.push(0x03);
        let mut m = Machine::from_bp(&b.finish()).unwrap();
        assert!(
            m.step()
                .unwrap_err()
                .to_string()
                .contains("BGI BP instruction")
        );
        assert_eq!(m.pc(), 0);
    }
}
