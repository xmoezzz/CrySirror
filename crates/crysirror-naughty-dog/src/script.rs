// SPDX-License-Identifier: MPL-2.0
//! The DC script-lambda interpreter (PPU `sub_1594E0` "Eval"; its tracing
//! disassembler `sub_154F00` names the opcodes).
//!
//! A `script-lambda` is `{code, statics, 'function}`. Instructions are big-
//! endian words `[op][a][b][c]`; registers are 32-bit words holding ints,
//! float bits or script addresses. Calls pass up to 16 arguments in
//! registers 24..40. `Call` (0x1c) invokes a native as
//! `fn(result*, argc, args[16])`; the native's status word before `args`
//! is 1 to suspend the lambda (waits; the frame chain is kept and resumed
//! at the next instruction) and 2 to abandon it (`go`). `CallFf` (0x1b)
//! pushes a frame for a DC lambda and its `Return` stores into the
//! caller's destination register. Exact native quirks are preserved: the
//! integer comparisons yield −1/0 but `IEqual` yields 1/0, `FEqual`
//! stores 1.0/0.0, `CastInteger`/`CastFloat` convert their destination in
//! place, `LoadStaticInt` indexes statics by a register while the `…Imm`
//! forms use the operand, and `IDivImm` divides the sign-extended value as
//! an unsigned 64-bit number.
use crate::dc::{Dc, TYPE_SCRIPT_LAMBDA};
use anyhow::{Context, Result, bail, ensure};

pub const ARG_BASE: usize = 24;
pub const MAX_ARGS: usize = 16;
/// Frame depth of the PPU frame pool is not established; this bounds
/// runaway recursion.
const MAX_DEPTH: usize = 64;
/// Instructions per `run` before the lambda is reported as runaway
/// (the PPU warns for long lambdas but has no limit).
const MAX_STEPS: u64 = 4_000_000;

/// Address range of native function descriptors returned by symbol lookup.
pub const NATIVE_BASE: u32 = 0xF000_0000;
/// The value lookup stores for an unknown symbol (`*(toc-0x7d80)` on the
/// PPU): `CallFf` through it reports an error and yields 0; `Call` through
/// it calls nothing.
pub const NULL_SYMBOL: u32 = 0xFFFF_FFF0;

/// What a native asks of the interpreter after returning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Continue,
    /// Status 1: suspend; resume at the next instruction.
    Suspend,
    /// Status 2: abandon the lambda (state change).
    Abort,
}

pub trait Natives {
    /// Run native `id` (an index below the descriptor base) with `argc`
    /// arguments.
    fn call(&mut self, dc: &mut Dc, id: u32, args: &[u32; MAX_ARGS], argc: u32) -> Result<(u32, Status)>;
    /// Resolve a symbol that is not defined by a loaded module to a
    /// native descriptor id.
    fn lookup(&self, sid: u32) -> Option<u32>;
    /// Record a script error the PPU prints and continues past.
    fn error(&mut self, message: String);
    /// Whether to print each instruction (diagnostic).
    fn trace_ops(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug)]
struct Frame {
    pc: u32,
    code: u32,
    statics: u32,
    argc: u32,
    /// Caller register receiving this frame's return value.
    ret: u8,
    regs: Box<[u32; 256]>,
}

/// A running or suspended lambda invocation.
#[derive(Clone, Debug)]
pub struct Thread {
    frames: Vec<Frame>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Returned(u32),
    Suspended,
    Aborted,
}

fn frame(dc: &Dc, lambda: u32, argc: u32, ret: u8) -> Result<Frame> {
    ensure!(dc.mem.type_of(lambda)? == TYPE_SCRIPT_LAMBDA, "call target {lambda:#x} is not a script-lambda");
    Ok(Frame { pc: 0, code: dc.mem.u32(lambda)?, statics: dc.mem.u32(lambda + 4)?, argc, ret, regs: Box::new([0; 256]) })
}

impl Thread {
    pub fn new(dc: &Dc, lambda: u32, args: &[u32]) -> Result<Self> {
        ensure!(args.len() <= MAX_ARGS, "too many script arguments");
        let mut f = frame(dc, lambda, args.len() as u32, 0)?;
        f.regs[ARG_BASE..ARG_BASE + args.len()].copy_from_slice(args);
        Ok(Self { frames: vec![f] })
    }

    /// Run until the outermost frame returns, a native suspends or the
    /// lambda is abandoned.
    pub fn run(&mut self, dc: &mut Dc, natives: &mut dyn Natives) -> Result<Outcome> {
        let mut steps = 0u64;
        loop {
            steps += 1;
            ensure!(steps <= MAX_STEPS, "script lambda exceeded {MAX_STEPS} instructions");
            let depth = self.frames.len();
            let f = self.frames.last_mut().context("script thread has no frame")?;
            let word = dc.mem.u32(f.code + 4 * f.pc)?;
            f.pc += 1;
            let (op, a, b, c) = ((word >> 24) as u8, (word >> 16) as u8 as usize, (word >> 8) as u8 as usize, word as u8 as usize);
            if natives.trace_ops() {
                eprintln!("[op {:#x}+{}] {op:#04x} {a} {b} {c}  r[a]={:#x} r[b]={:#x} r[c]={:#x}", f.code, f.pc - 1, f.regs[a], f.regs[b], f.regs[c]);
            }
            let r = &mut f.regs;
            let fl = |v: u32| f32::from_bits(v);
            let set_bool = |r: &mut [u32; 256], v: bool| r[a] = if v { u32::MAX } else { 0 };
            match op {
                0x00 => {
                    let value = r[b];
                    let done = self.frames.pop().expect("frame");
                    match self.frames.last_mut() {
                        None => return Ok(Outcome::Returned(value)),
                        Some(caller) => caller.regs[done.ret as usize] = value,
                    }
                }
                0x01 => r[a] = r[b].wrapping_add(r[c]),
                0x02 => r[a] = r[b].wrapping_sub(r[c]),
                0x03 => r[a] = r[b].wrapping_mul(r[c]),
                0x04 => r[a] = (r[b] as i32).checked_div(r[c] as i32).unwrap_or(0) as u32,
                0x05 => r[a] = (fl(r[b]) + fl(r[c])).to_bits(),
                0x06 => r[a] = (fl(r[b]) - fl(r[c])).to_bits(),
                0x07 => r[a] = (fl(r[b]) * fl(r[c])).to_bits(),
                0x08 => r[a] = (fl(r[b]) / fl(r[c])).to_bits(),
                // LoadStaticInt/Float/Pointer: index from a register.
                0x09..=0x0b => r[a] = dc.mem.u32(f.statics + 4 * r[b])?,
                0x0c => r[a] = (c << 8 | b) as u32,
                // LoadU32/LoadFloat/LoadPointer.
                0x0d..=0x0f => r[a] = dc.mem.u32(r[b])?,
                // StoreInt/Float/Pointer: *reg[b] = reg[c]; reg[a] = reg[c].
                0x10..=0x12 => {
                    let (to, v) = (r[b], r[c]);
                    dc.mem.write_u32(to, v)?;
                    r[a] = v;
                }
                0x13..=0x15 => {
                    let name = dc.mem.u32(f.statics + 4 * b as u32)?;
                    r[a] = match natives.lookup(name) {
                        Some(id) => NATIVE_BASE + id,
                        None => match dc.symbols.get(&name) {
                            Some(v) => *v,
                            None => {
                                natives.error(format!("unbound script symbol {name:#010x}"));
                                NULL_SYMBOL
                            }
                        },
                    };
                }
                0x16..=0x18 => r[a] = r[b],
                0x19 => r[a] = fl(r[a]) as i32 as u32,
                0x1a => r[a] = (r[a] as i32 as f32).to_bits(),
                0x1b => {
                    let target = r[b];
                    if target == NULL_SYMBOL || target >= NATIVE_BASE {
                        natives.error(format!("CallFf through {target:#010x}"));
                        r[a] = 0;
                        continue;
                    }
                    ensure!(depth < MAX_DEPTH, "script call depth");
                    let args: [u32; MAX_ARGS] = r[ARG_BASE..ARG_BASE + MAX_ARGS].try_into()?;
                    let mut callee = frame(dc, target, c as u32, a as u8)?;
                    callee.regs[ARG_BASE..ARG_BASE + c.min(MAX_ARGS)].copy_from_slice(&args[..c.min(MAX_ARGS)]);
                    self.frames.push(callee);
                }
                0x1c => {
                    let target = r[b];
                    if target == 0 || target == NULL_SYMBOL {
                        continue;
                    }
                    ensure!(target >= NATIVE_BASE, "Call through non-native {target:#010x}");
                    let args: [u32; MAX_ARGS] = r[ARG_BASE..ARG_BASE + MAX_ARGS].try_into()?;
                    let (value, status) = natives.call(dc, target - NATIVE_BASE, &args, c as u32)?;
                    if crate::ss::trace_natives().is_some() {
                        eprintln!("[native result] {value:#010x} {status:?}");
                    }
                    self.frames.last_mut().expect("frame").regs[a] = value;
                    match status {
                        Status::Continue => {}
                        Status::Suspend => return Ok(Outcome::Suspended),
                        Status::Abort => {
                            self.frames.clear();
                            return Ok(Outcome::Aborted);
                        }
                    }
                }
                0x1d => r[a] = u32::from(r[b] == r[c]),
                0x1e => set_bool(r, (r[b] as i32) > (r[c] as i32)),
                0x1f => set_bool(r, (r[b] as i32) >= (r[c] as i32)),
                0x20 => set_bool(r, (r[b] as i32) < (r[c] as i32)),
                0x21 => set_bool(r, (r[b] as i32) <= (r[c] as i32)),
                0x22 => r[a] = if fl(r[b]) - fl(r[c]) == 0. { 1f32 } else { 0f32 }.to_bits(),
                0x23 => set_bool(r, fl(r[b]) > fl(r[c])),
                0x24 => set_bool(r, !(fl(r[b]) < fl(r[c]))),
                0x25 => set_bool(r, !(fl(r[b]) >= fl(r[c]))),
                0x26 => set_bool(r, !(fl(r[b]) > fl(r[c]))),
                0x27 => r[a] = (r[b] as i32).checked_rem(r[c] as i32).unwrap_or(0) as u32,
                0x28 => r[a] = (fl(r[b]) % fl(r[c])).to_bits(),
                0x29 => r[a] = (r[b] as i32).wrapping_abs() as u32,
                0x2a => r[a] = fl(r[b]).abs().to_bits(),
                0x2d => f.pc = (c << 8 | a) as u32,
                0x2e => {
                    if r[b] != 0 {
                        f.pc = (c << 8 | a) as u32;
                    }
                }
                0x2f => {
                    if r[b] == 0 {
                        f.pc = (c << 8 | a) as u32;
                    }
                }
                0x30 => set_bool(r, r[b] == 0),
                0x31 => r[a] = r[b] & r[c],
                0x32 => r[a] = !r[b],
                0x33 => r[a] = r[b] | r[c],
                0x34 => r[a] = r[b] ^ r[c],
                0x35 => r[a] = !(r[b] | r[c]),
                0x36 => set_bool(r, r[b] != 0 && r[c] != 0),
                0x37 => set_bool(r, r[b] != 0 || r[c] != 0),
                0x38 => r[a] = r[b].wrapping_neg(),
                0x39 => r[a] = (-fl(r[b])).to_bits(),
                0x3a => r[a] = f.argc,
                0x3b => r[a] = r[b].wrapping_add(c as u32),
                0x3c => r[a] = r[b].wrapping_sub(c as u32),
                0x3d => r[a] = r[b].wrapping_mul(c as u32),
                0x3e => r[a] = if c == 0 { 0 } else { ((r[b] as i32 as i64 as u64) / c as u64) as u32 },
                0x3f..=0x41 => r[a] = dc.mem.u32(f.statics + 4 * b as u32)?,
                _ => bail!("Eval: Unknown opcode '{op:#x}' encountered"),
            }
        }
    }
}

/// The statics address and instruction words of a lambda, for tools.
pub fn lambda_code(dc: &Dc, lambda: u32) -> Result<(u32, Vec<u32>)> {
    let (code, statics) = (dc.mem.u32(lambda)?, dc.mem.u32(lambda + 4)?);
    // The statics array's type word follows the code.
    let n = statics.checked_sub(code + 4).context("lambda statics precede code")? / 4;
    Ok((statics, (0..n).map(|i| dc.mem.u32(code + 4 * i)).collect::<Result<_>>()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dc::{TYPE_ARRAY, TYPE_SCRIPT_FILE, sid};

    struct Log(Vec<(u32, Vec<u32>)>);
    impl Natives for Log {
        fn call(&mut self, _: &mut Dc, id: u32, args: &[u32; MAX_ARGS], argc: u32) -> Result<(u32, Status)> {
            self.0.push((id, args[..argc as usize].to_vec()));
            Ok((7, if id == 1 { Status::Suspend } else { Status::Continue }))
        }
        fn lookup(&self, s: u32) -> Option<u32> {
            [sid("n0"), sid("n1")].iter().position(|n| *n == s).map(|i| i as u32)
        }
        fn error(&mut self, m: String) {
            panic!("{m}");
        }
    }

    /// Module with one lambda: statics [n0, n1, 5]; code calls n0(5),
    /// n1(), then returns r(n0 result) + 3 via IAddImm.
    fn module(code: &[u32]) -> Vec<u8> {
        let statics = [sid("n0"), sid("n1"), 5];
        let lam = 0x2cu32;
        let code_at = lam + 12 + 4;
        let statics_at = code_at + 4 * code.len() as u32 + 4;
        let mut w = vec![u32::from_be_bytes(*b"DC00"), 1, 0, 0, 0x18, TYPE_SCRIPT_FILE, 1, 1, sid("f"), lam, TYPE_SCRIPT_LAMBDA, code_at, statics_at, sid("function"), TYPE_ARRAY];
        w.extend_from_slice(code);
        w.push(TYPE_ARRAY);
        w.extend_from_slice(&statics);
        let size = 4 * w.len() as u32;
        w[2] = size;
        w.extend_from_slice(&[4, 0x10, 0x24, 0x2c, 0x30]);
        w.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    #[test]
    fn calls_suspends_and_returns() {
        let ins = |op: u32, a: u32, b: u32, c: u32| op << 24 | a << 16 | b << 8 | c;
        let code = [
            ins(0x15, 0, 0, 0),  // r0 = lookup n0
            ins(0x3f, 1, 2, 0),  // r1 = statics[2] = 5
            ins(0x16, 24, 1, 0), // arg0 = r1
            ins(0x1c, 2, 0, 1),  // r2 = n0(r1)
            ins(0x15, 0, 1, 0),  // r0 = lookup n1
            ins(0x1c, 3, 0, 0),  // n1() suspends
            ins(0x3b, 4, 2, 3),  // r4 = r2 + 3
            ins(0x1e, 5, 4, 1),  // r5 = r4 > r1 → −1
            ins(0x2f, 10, 5, 0), // if !r5 goto 10
            ins(0x00, 0, 4, 0),  // return r4
            ins(0x00, 0, 1, 0),
        ];
        let mut dc = Dc::default();
        dc.load(sid("m"), &module(&code)).unwrap();
        let mut log = Log(Vec::new());
        let mut t = Thread::new(&dc, dc.symbols[&sid("f")], &[]).unwrap();
        assert_eq!(t.run(&mut dc, &mut log).unwrap(), Outcome::Suspended);
        assert_eq!(log.0, vec![(0, vec![5]), (1, vec![])]);
        assert_eq!(t.run(&mut dc, &mut log).unwrap(), Outcome::Returned(10));
    }
}
