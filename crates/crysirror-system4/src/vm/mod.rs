// Original AIN VM semantics researched against xsystem4 a85467d78eb5c7fb6e581d7fe5a6fdaec42b9ef7.
// Copyright (C) 2019 Nunuhara Cabbage; Rust adoption (C) 2026 CrySirror contributors.
// GPL-2.0-or-later. Native values/pages only; no original HLL engine is linked.
//! Bounded, resumable AIN v5 interpreter. External calls leave the PC, arguments
//! and heap untouched until a thin host adapter explicitly commits completion.
//! Cloning captures native execution state; this is not an original-save codec.
pub mod external;
pub mod heap;
use crate::ain::{InitialValue, Program, Variable};
use anyhow::{Context, Result, bail, ensure};
use heap::{Heap, Object, Page, PageKind, element_kind, is_array, is_reference, owns_heap};
use std::sync::Arc;
const MAX_STACK: usize = 65536;
const MAX_CALLS: usize = 4096;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalTarget {
    System(i32),
    Hll { library: usize, function: usize },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalCall {
    pub pc: u32,
    pub next: u32,
    pub target: ExternalTarget,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Advanced,
    External(ExternalCall),
    Halted,
}
#[derive(Clone, Debug)]
enum Return {
    Address(u32),
    Actions,
}
#[derive(Clone, Debug)]
pub struct Frame {
    pub function: usize,
    pub local_page: i32,
    pub structure_page: i32,
    pub call_pc: u32,
    return_to: Return,
}
#[derive(Clone, Debug)]
enum Action {
    Call(usize, i32),
    InitGlobal(usize),
    InitStruct {
        index: i32,
        slot: i32,
        member: usize,
        depth: usize,
    },
    DeleteMembers {
        slot: i32,
        member: usize,
        call_destructors: bool,
    },
    Array(ArrayBuild),
    Release(i32),
    ExitRelease(i32),
    Free(i32),
    Return,
    Resume(u32),
    Finish,
}
#[derive(Clone, Debug)]
struct ArrayBuild {
    dimensions: Vec<i32>,
    kind: i32,
    structure: i32,
    destination: i32,
    next: u32,
    levels: Vec<Page>,
}
#[derive(Clone, Debug)]
pub struct Machine {
    pub program: Arc<Program>,
    pub heap: Heap,
    pub stack: Vec<i32>,
    pub frames: Vec<Frame>,
    pub pc: u32,
    pub instructions: u64,
    pub entered_main: bool,
    actions: Vec<Action>,
    running_actions: bool,
    halted: bool,
    pending: Option<ExternalCall>,
}
impl Machine {
    pub fn new(program: Arc<Program>) -> Result<Self> {
        ensure!(program.version == 5, "System4 VM supports AIN v5 only");
        let mut m = Self {
            heap: Heap::new(program.globals.len())?,
            program,
            stack: Vec::new(),
            frames: Vec::new(),
            pc: 0,
            instructions: 0,
            entered_main: false,
            actions: Vec::new(),
            running_actions: true,
            halted: false,
            pending: None,
        };
        // All storage exists before any constructor runs (cross-global references).
        for i in 0..m.program.globals.len() {
            let v = m.program.globals[i].clone();
            let x = if v.kind == 13 {
                m.alloc_structure(v.structure, 0)?
            } else {
                m.initial(v.kind)?
            };
            m.heap.set(0, i as i32, x)?;
        }
        for init in m.program.initial_values.clone() {
            match init {
                InitialValue::Integer { global, value, .. } => {
                    m.heap.set(0, global as i32, value)?
                }
                InitialValue::String { global, value } => {
                    let x = m.heap.alloc(Object::String(Arc::new(value)))?;
                    m.heap.set(0, global as i32, x)?;
                }
            }
        }
        for init in m.program.initial_values.clone() {
            if let InitialValue::Integer { global, value, .. } = init {
                let k = m.program.globals[global as usize].kind;
                if value != -1 && is_reference(k) {
                    if matches!(k, 18 | 19 | 31 | 51 | 59) {
                        m.heap.set(0, global as i32, 0)?;
                        m.heap.set(0, global as i32 + 1, value)?;
                        m.heap.reference(0)?;
                    } else {
                        let x = m.heap.get(0, value)?;
                        m.heap.set(0, global as i32, x)?;
                        m.heap.reference(x)?;
                    }
                }
            }
        }
        let mut startup = Vec::new();
        if let Some(f) = m.program.functions.iter().position(|f| f.name == b"0") {
            startup.push(Action::Call(f, -1));
        }
        for i in 0..m.program.globals.len() {
            let g = &m.program.globals[i];
            if g.kind == 13 {
                startup.push(Action::InitGlobal(i));
            }
        }
        ensure!(
            (m.program.main as usize) < m.program.functions.len(),
            "System4 main function index"
        );
        startup.push(Action::Call(m.program.main as usize, -1));
        startup.push(Action::Finish);
        m.actions = startup.into_iter().rev().collect();
        Ok(m)
    }
    fn initial(&mut self, k: i32) -> Result<i32> {
        Ok(match k {
            12 => self.heap.alloc(Object::String(Arc::new(Vec::new())))?,
            13 => -1,
            k if is_reference(k) => -1,
            k if is_array(k) || k == 63 => self.heap.alloc(Object::Page(None))?,
            0 | 10 | 11 | 26 | 27 | 47 | 55 => 0,
            _ => bail!("System4 unsupported native variable type {k}"),
        })
    }
    fn alloc_structure(&mut self, index: i32, depth: usize) -> Result<i32> {
        ensure!(depth < 64, "System4 structure nesting budget");
        let i = usize::try_from(index)?;
        let members = self
            .program
            .structures
            .get(i)
            .context("System4 structure index")?
            .members
            .clone();
        let slot = self.heap.alloc(Object::Page(Some(Page {
            kind: PageKind::Structure(i),
            values: vec![0; members.len()],
        })))?;
        for (j, v) in members.iter().enumerate() {
            let x = if v.kind == 13 {
                self.alloc_structure(v.structure, depth + 1)?
            } else {
                self.initial(v.kind)?
            };
            self.heap.set(slot, j as i32, x)?;
        }
        Ok(slot)
    }
    fn local(&self) -> Result<i32> {
        Ok(self
            .frames
            .last()
            .context("System4 no call frame")?
            .local_page)
    }
    fn structure(&self) -> Result<i32> {
        let s = self
            .frames
            .last()
            .context("System4 no call frame")?
            .structure_page;
        self.heap.page(s)?;
        Ok(s)
    }
    fn variable(&self, page: i32, index: i32) -> Result<Variable> {
        let i = usize::try_from(index)?;
        let p = self.heap.page(page)?;
        p.values.get(i).context("System4 variable index")?;
        Ok(match p.kind {
            PageKind::Global => self.program.globals.get(i),
            PageKind::Local(f) => self.program.functions[f].variables.get(i),
            PageKind::Structure(s) => self.program.structures[s].members.get(i),
            PageKind::Array {
                kind,
                structure,
                rank,
            } => {
                return Ok(Variable {
                    name: Vec::new(),
                    kind: if rank > 1 { kind } else { element_kind(kind)? },
                    structure,
                    rank: rank.saturating_sub(1) as i32,
                    group: None,
                });
            }
        }
        .context("System4 variable metadata")?
        .clone())
    }
    fn peek(&self, n: usize) -> Result<i32> {
        self.stack
            .len()
            .checked_sub(n + 1)
            .and_then(|i| self.stack.get(i))
            .copied()
            .context("System4 operand stack underflow")
    }
    fn reference(&self, n: usize) -> Result<(i32, i32)> {
        let index = self.peek(n)?;
        let page = self.peek(n + 1)?;
        self.heap.get(page, index)?;
        Ok((page, index))
    }
    fn push(&mut self, v: i32) -> Result<()> {
        ensure!(self.stack.len() < MAX_STACK, "System4 operand stack budget");
        self.stack.push(v);
        Ok(())
    }
    fn consume(&mut self, n: usize) -> Result<()> {
        ensure!(n <= self.stack.len(), "System4 operand stack underflow");
        self.stack.truncate(self.stack.len() - n);
        Ok(())
    }
    fn call(&mut self, function: usize, structure: i32, return_to: Return) -> Result<()> {
        ensure!(self.frames.len() < MAX_CALLS, "System4 call stack budget");
        let f = self
            .program
            .functions
            .get(function)
            .context("System4 function index")?
            .clone();
        let argc = f.arguments as usize;
        ensure!(
            argc <= f.variables.len() && argc <= self.stack.len(),
            "System4 native argument count"
        );
        if structure != -1 {
            self.heap.page(structure)?;
        }
        // The original allocates the local heap slot before its string/array locals.
        let slot = self.heap.alloc(Object::Page(Some(Page {
            kind: PageKind::Local(function),
            values: vec![0; f.variables.len()],
        })))?;
        for (i, v) in f.variables.iter().enumerate().skip(argc) {
            let x = self.initial(v.kind)?;
            self.heap.set(slot, i as i32, x)?;
        }
        for i in 0..argc {
            let x = self.stack[self.stack.len() - argc + i];
            if is_reference(f.variables[i].kind) {
                self.heap.reference(x)?;
            }
            self.heap.set(slot, i as i32, x)?;
        }
        self.consume(argc)?;
        self.frames.push(Frame {
            function,
            local_page: slot,
            structure_page: structure,
            call_pc: self.pc,
            return_to,
        });
        self.pc = f.address;
        self.entered_main |= function == self.program.main as usize;
        Ok(())
    }
    fn schedule(&mut self, mut ordered: Vec<Action>, next: u32) {
        ordered.push(Action::Resume(next));
        self.actions.extend(ordered.into_iter().rev());
        self.running_actions = true;
    }
    fn action(&mut self) -> Result<()> {
        let a = self.actions.pop().context("System4 empty continuation")?;
        match a {
            Action::Call(f, s) => {
                self.call(f, s, Return::Actions)?;
                self.running_actions = false;
            }
            Action::Resume(pc) => {
                self.pc = pc;
                self.running_actions = false;
            }
            Action::Finish => {
                self.halted = true;
                self.running_actions = false;
            }
            Action::Free(slot) => self.heap.free(slot)?,
            Action::Release(slot) => self.release(slot, true)?,
            Action::ExitRelease(slot) => self.release(slot, false)?,
            Action::DeleteMembers {
                slot,
                member,
                call_destructors,
            } => {
                if member > 0 {
                    let i = member - 1;
                    self.actions.push(Action::DeleteMembers {
                        slot,
                        member: i,
                        call_destructors,
                    });
                    if owns_heap(self.variable(slot, i as i32)?.kind) {
                        let value = self.heap.get(slot, i as i32)?;
                        self.actions.push(if call_destructors {
                            Action::Release(value)
                        } else {
                            Action::ExitRelease(value)
                        });
                    }
                }
            }
            Action::InitGlobal(i) => {
                let g = &self.program.globals[i];
                self.actions.push(Action::InitStruct {
                    index: g.structure,
                    slot: self.heap.get(0, i as i32)?,
                    member: 0,
                    depth: 0,
                });
            }
            Action::InitStruct {
                index,
                slot,
                member,
                depth,
            } => {
                ensure!(depth < 64, "System4 constructor depth budget");
                let st = self
                    .program
                    .structures
                    .get(usize::try_from(index)?)
                    .context("System4 constructor structure")?;
                if member < st.members.len() {
                    let v = st.members[member].clone();
                    self.actions.push(Action::InitStruct {
                        index,
                        slot,
                        member: member + 1,
                        depth,
                    });
                    if v.kind == 13 {
                        let child = self.heap.get(slot, member as i32)?;
                        self.actions.push(Action::InitStruct {
                            index: v.structure,
                            slot: child,
                            member: 0,
                            depth: depth + 1,
                        });
                    }
                } else if st.constructor > 0 {
                    self.actions
                        .push(Action::Call(st.constructor as usize, slot));
                }
            }
            Action::Array(build) => self.array_step(build)?,
            Action::Return => {
                let f = self.frames.pop().context("System4 return without frame")?;
                match f.return_to {
                    Return::Address(pc) => {
                        self.pc = pc;
                        self.running_actions = false;
                    }
                    Return::Actions => {}
                }
            }
        }
        Ok(())
    }
    fn release(&mut self, slot: i32, call_destructors: bool) -> Result<()> {
        if slot == -1 {
            return Ok(());
        }
        let refs = self.heap.slot(slot)?.references;
        if refs > 1 {
            self.heap.slot_mut(slot)?.references -= 1;
            return Ok(());
        }
        let object = self.heap.slot(slot)?.object.clone();
        self.actions.push(Action::Free(slot));
        if let Object::Page(Some(page)) = object {
            self.actions.push(Action::DeleteMembers {
                slot,
                member: page.values.len(),
                call_destructors,
            });
            if call_destructors {
                if let PageKind::Structure(s) = page.kind {
                    let d = self.program.structures[s].destructor;
                    if d > 0 {
                        self.actions.push(Action::Call(d as usize, slot));
                    }
                }
            }
        }
        Ok(())
    }
    pub fn step(&mut self) -> Result<Step> {
        if self.halted {
            return Ok(Step::Halted);
        }
        if let Some(c) = &self.pending {
            return Ok(Step::External(c.clone()));
        }
        if self.running_actions {
            self.action()?;
            return Ok(if self.halted {
                Step::Halted
            } else {
                Step::Advanced
            });
        }
        let instruction = self.program.instruction(self.pc)?;
        let pc = self.pc;
        let result = self
            .execute(&instruction)
            .with_context(|| format!("System4 {} pc={pc:#x}", instruction.name))?;
        if let Some(call) = result {
            self.pending = Some(call.clone());
            return Ok(Step::External(call));
        }
        self.instructions += 1;
        Ok(Step::Advanced)
    }
    fn execute(&mut self, i: &crate::instruction::Instruction) -> Result<Option<ExternalCall>> {
        let a = &i.arguments;
        let mut next = i.next;
        match i.name {
            "PUSH" | "F_PUSH" => self.push(a[0])?,
            "POP" => self.consume(1)?,
            "FUNC" => {}
            "PUSHGLOBALPAGE" => self.push(0)?,
            "PUSHLOCALPAGE" => self.push(self.local()?)?,
            "PUSHSTRUCTPAGE" => self.push(self.structure()?)?,
            "SH_GLOBALREF" | "SH_LOCALREF" | "SH_STRUCTREF" => {
                let p = match i.name {
                    "SH_GLOBALREF" => 0,
                    "SH_LOCALREF" => self.local()?,
                    _ => self.structure()?,
                };
                self.push(self.heap.get(p, a[0])?)?;
            }
            "REF" | "S_REF" | "SR_REF" => {
                let (p, v) = self.reference(0)?;
                let x = self.heap.get(p, v)?;
                let x = match i.name {
                    "S_REF" => self.heap.alloc(Object::String(self.heap.string(x)?))?,
                    "SR_REF" => self.copy_object(x, 0)?,
                    _ => x,
                };
                self.consume(2)?;
                self.push(x)?;
            }
            "REFREF" => {
                let (p, v) = self.reference(0)?;
                let x = self.heap.get(p, v)?;
                let y = self.heap.get(p, v + 1)?;
                self.consume(2)?;
                self.push(x)?;
                self.push(y)?;
            }
            "ASSIGN" | "F_ASSIGN" => {
                let x = self.peek(0)?;
                let (p, v) = self.reference(1)?;
                self.heap.set(p, v, x)?;
                self.consume(3)?;
                self.push(x)?;
            }
            "R_ASSIGN" => {
                let si = self.peek(0)?;
                let sp = self.peek(1)?;
                let (dp, di) = self.reference(2)?;
                self.heap.get(dp, di + 1)?;
                self.heap.set(dp, di, sp)?;
                self.heap.set(dp, di + 1, si)?;
                self.consume(4)?;
                self.push(sp)?;
                self.push(si)?;
            }
            "S_PUSH" => {
                let s = self
                    .program
                    .strings
                    .get(usize::try_from(a[0])?)
                    .context("System4 string constant")?
                    .clone();
                let slot = self.heap.alloc(Object::String(Arc::new(s)))?;
                self.push(slot)?;
            }
            "S_ASSIGN" => {
                let r = self.peek(0)?;
                let l = self.peek(1)?;
                let s = self.heap.string(r)?;
                self.heap.set_string(l, s)?;
                self.consume(2)?;
                self.push(r)?;
            }
            "S_MOD" => {
                use crysirror_text::format::{format_one_bytes,ByteFormatValue};
                let kind=self.peek(0)?;let value=self.peek(1)?;let fmt=self.peek(2)?;
                let template=self.heap.string(fmt)?;
                let string=if kind==4 {Some(self.heap.string(value)?)}else{None};
                let argument=match kind {
                    2|48|56=>ByteFormatValue::Integer(value),
                    4=>ByteFormatValue::Bytes(string.as_ref().unwrap().as_slice()),
                    _=>bail!("System4 S_MOD format value type {kind} unsupported"),
                };
                let output=format_one_bytes(template.as_slice(),argument)?;
                let release_string=kind==4&&output.matched;
                let slot=self.heap.alloc(Object::String(Arc::new(output.bytes)))?;
                self.consume(3)?;self.push(slot)?;
                let mut release=vec![Action::Release(fmt)];
                if release_string {release.push(Action::Release(value));}
                self.schedule(release,next);return Ok(None);
            }
            "S_ADD" => {
                let r = self.peek(0)?;
                let l = self.peek(1)?;
                let mut s = (*self.heap.string(l)?).clone();
                let b = self.heap.string(r)?;
                ensure!(
                    s.len().checked_add(b.len()).is_some_and(|n| n < 4_000_000),
                    "System4 string budget"
                );
                s.extend(&**b);
                let x = self.heap.alloc(Object::String(Arc::new(s)))?;
                self.consume(2)?;
                self.push(x)?;
                self.schedule(vec![Action::Release(l), Action::Release(r)], next);
                return Ok(None);
            }
            "S_POP" | "SR_POP" | "DELETE" => {
                let slot = self.peek(0)?;
                self.heap.slot(slot).or_else(|e| {
                    if slot == -1 {
                        Ok(self.heap.slot(0)?)
                    } else {
                        Err(e)
                    }
                })?;
                self.consume(1)?;
                self.schedule(vec![Action::Release(slot)], next);
                return Ok(None);
            }
            "SP_INC" => {
                let slot = self.peek(0)?;
                self.heap.reference(slot)?;
                self.consume(1)?;
            }
            "NEW" | "SH_LOCALCREATE" => {
                let (s, dst) = if i.name == "NEW" {
                    (self.peek(0)?, None)
                } else {
                    (a[1], Some((self.local()?, a[0])))
                };
                let slot = self.alloc_structure(s, 0)?;
                let tasks = vec![Action::InitStruct {
                    index: s,
                    slot,
                    member: 0,
                    depth: 0,
                }];
                if let Some((p, v)) = dst {
                    self.heap.set(p, v, slot)?;
                } else {
                    self.consume(1)?;
                    self.push(slot)?;
                }
                self.schedule(tasks, next);
                return Ok(None);
            }
            "CALLFUNC" => {
                self.call(usize::try_from(a[0])?, -1, Return::Address(next))?;
                return Ok(None);
            }
            "CALLMETHOD" => {
                let f = usize::try_from(a[0])?;
                let n = self
                    .program
                    .functions
                    .get(f)
                    .context("System4 method")?
                    .arguments as usize;
                let s = self.peek(n)?;
                self.heap.page(s)?;
                self.stack.remove(self.stack.len() - n - 1);
                self.call(f, s, Return::Address(next))?;
                return Ok(None);
            }
            "RETURN" => {
                let slot = self.local()?;
                self.actions.push(Action::Return);
                self.actions.push(Action::Release(slot));
                self.running_actions = true;
                return Ok(None);
            }
            "CALLHLL" => {
                let lib = usize::try_from(a[0])?;
                let fun = usize::try_from(a[1])?;
                self.program
                    .libraries
                    .get(lib)
                    .and_then(|l| l.functions.get(fun))
                    .context("System4 HLL declaration index")?;
                return Ok(Some(ExternalCall {
                    pc: i.pc,
                    next,
                    target: ExternalTarget::Hll {
                        library: lib,
                        function: fun,
                    },
                }));
            }
            "CALLSYS" => {
                return Ok(Some(ExternalCall {
                    pc: i.pc,
                    next,
                    target: ExternalTarget::System(a[0]),
                }));
            }
            "JUMP" => next = u32::try_from(a[0])?,
            "IFZ" | "IFNZ" => {
                let x = self.peek(0)?;
                if (x == 0) == (i.name == "IFZ") {
                    next = u32::try_from(a[0])?;
                }
                self.consume(1)?;
            }
            "SWITCH" => {
                let s = self
                    .program
                    .switches
                    .get(usize::try_from(a[0])?)
                    .context("System4 switch")?;
                let x = self.peek(0)?;
                next = s
                    .cases
                    .iter()
                    .find(|(v, _)| *v == x)
                    .map(|(_, p)| *p)
                    .unwrap_or(if s.default > 0 {
                        s.default as u32
                    } else {
                        next
                    });
                self.consume(1)?;
            }
            "DUP" | "DUP_U2" => {
                let n = usize::from(i.name == "DUP_U2");
                self.push(self.peek(n)?)?;
            }
            "DUP2" => {
                let x = self.peek(1)?;
                let y = self.peek(0)?;
                self.push(x)?;
                self.push(y)?;
            }
            "SWAP" => {
                self.peek(1)?;
                let n = self.stack.len();
                self.stack.swap(n - 1, n - 2);
            }
            "DUP_X2" | "DUP2_X1" => {
                self.peek(2)?;
                ensure!(self.stack.len() + 2 <= MAX_STACK, "System4 stack budget");
                let n = self.stack.len();
                let p = self.stack[n - 3..].to_vec();
                self.consume(3)?;
                self.stack.extend(if i.name == "DUP_X2" {
                    vec![p[2], p[0], p[1], p[2]]
                } else {
                    vec![p[1], p[2], p[0], p[1], p[2]]
                });
            }
            "INV" | "NOT" | "COMPL" | "ITOB" | "ITOLI" | "ITOF" | "FTOI" | "F_INV" => {
                let x = self.peek(0)?;
                let y = match i.name {
                    "INV" => x.wrapping_neg(),
                    "NOT" => (x == 0) as i32,
                    "COMPL" => !x,
                    "ITOB" => (x != 0) as i32,
                    "ITOLI" => x.max(0),
                    "ITOF" => (x as f32).to_bits() as i32,
                    "FTOI" => {
                        let f = f32::from_bits(x as u32);
                        ensure!(
                            f.is_finite() && f >= i32::MIN as f32 && f < 2147483648.0,
                            "System4 float conversion outside native range"
                        );
                        f as i32
                    }
                    _ => (-f32::from_bits(x as u32)).to_bits() as i32,
                };
                *self.stack.last_mut().unwrap() = y;
            }
            "ADD" | "SUB" | "MUL" | "DIV" | "MOD" | "AND" | "OR" | "XOR" | "LSHIFT" | "RSHIFT"
            | "LT" | "GT" | "LTE" | "GTE" | "NOTE" | "EQUALE" | "LI_ADD" | "LI_SUB" | "LI_MUL"
            | "LI_DIV" | "LI_MOD" | "F_ADD" | "F_SUB" | "F_MUL" | "F_DIV" | "F_LT" | "F_GT"
            | "F_LTE" | "F_GTE" | "F_NOTE" | "F_EQUALE" => {
                let y = self.peek(0)?;
                let x = self.peek(1)?;
                let v = calculate(i.name, x, y)?;
                self.consume(2)?;
                self.push(v)?;
            }
            "PLUSA" | "MINUSA" | "MULA" | "DIVA" | "MODA" | "ANDA" | "ORA" | "XORA" | "LSHIFTA"
            | "RSHIFTA" => {
                let y = self.peek(0)?;
                let (p, v) = self.reference(1)?;
                let x = self.heap.get(p, v)?;
                // Original conditional expression skips stack_pop_var at zero.
                if i.name == "DIVA" && y == 0 {
                    self.consume(1)?;
                    self.push(0)?;
                    self.pc = next;
                    return Ok(None);
                }
                let op = match i.name {
                    "PLUSA" => "ADD",
                    "MINUSA" => "SUB",
                    "MULA" => "MUL",
                    "DIVA" => "DIV",
                    "MODA" => {
                        ensure!(y != 0, "System4 MODA zero divisor");
                        "MOD"
                    }
                    "ANDA" => "AND",
                    "ORA" => "OR",
                    "XORA" => "XOR",
                    "LSHIFTA" => "LSHIFT",
                    _ => "RSHIFT",
                };
                let x = calculate(op, x, y)?;
                self.heap.set(p, v, x)?;
                self.consume(3)?;
                self.push(x)?;
            }
            "INC" | "DEC" => {
                let (p, v) = self.reference(0)?;
                let x = self.heap.get(p, v)?;
                self.heap
                    .set(p, v, x.wrapping_add(if i.name == "INC" { 1 } else { -1 }))?;
                self.consume(2)?;
            }
            "SH_LOCALASSIGN" => self.heap.set(self.local()?, a[0], a[1])?,
            "SH_LOCALINC" | "SH_LOCALDEC" => {
                let p = self.local()?;
                let x = self.heap.get(p, a[0])?;
                self.heap.set(
                    p,
                    a[0],
                    x.wrapping_add(if i.name == "SH_LOCALINC" { 1 } else { -1 }),
                )?;
            }
            "SH_MEM_ASSIGN_IMM" => self.heap.set(self.structure()?, a[0], a[1])?,
            "SH_MEM_ASSIGN_LOCAL" => {
                self.heap
                    .set(self.structure()?, a[0], self.heap.get(self.local()?, a[1])?)?
            }
            "A_ALLOC" => {
                let rank = usize::try_from(self.peek(0)?)?;
                ensure!((1..=32).contains(&rank), "System4 array rank budget");
                let (p, v) = self.reference(rank + 1)?;
                let array = self.heap.get(p, v)?;
                let var = self.variable(p, v)?;
                ensure!(is_array(var.kind), "System4 A_ALLOC variable not array");
                ensure!(
                    self.heap.page_opt(array)?.is_none(),
                    "System4 replacing allocated array not yet supported"
                );
                let mut dims = Vec::new();
                for n in (1..=rank).rev() {
                    dims.push(self.peek(n)?);
                }
                // Rank is popped before alloc_array. The remaining reference/dimensions
                // stay below constructor calls until the entire root has completed.
                self.consume(1)?;
                let page = Page {
                    kind: PageKind::Array {
                        kind: var.kind,
                        structure: var.structure,
                        rank,
                    },
                    values: Vec::new(),
                };
                self.actions.push(Action::Array(ArrayBuild {
                    dimensions: dims,
                    kind: var.kind,
                    structure: var.structure,
                    destination: array,
                    next,
                    levels: vec![page],
                }));
                self.running_actions = true;
                return Ok(None);
            }
            "A_NUMOF" => {
                let rank = self.peek(0)?;
                let (p, v) = self.reference(1)?;
                let array = self.heap.get(p, v)?;
                let n = self.array_numof(array, rank)?;
                self.consume(3)?;
                self.push(n)?;
            }
            "A_EMPTY" => {
                let (p, v) = self.reference(0)?;
                let array = self.heap.get(p, v)?;
                let empty = self
                    .heap
                    .page_opt(array)?
                    .is_none_or(|p| p.values.is_empty());
                self.consume(2)?;
                self.push(empty as i32)?;
            }
            "SR_REF2" | "A_REF" => {
                let x = self.peek(0)?;
                let copy = self.copy_object(x, 0)?;
                self.consume(1)?;
                self.push(copy)?;
            }
            _ => bail!(
                "unsupported original instruction {} opcode={:#x}",
                i.name,
                i.opcode
            ),
        }
        self.pc = next;
        Ok(None)
    }
    fn array_step(&mut self, mut b: ArrayBuild) -> Result<()> {
        let level = b.levels.len() - 1;
        let n = b.dimensions[level].max(0) as usize;
        ensure!(n <= heap::MAX_VALUES, "System4 array dimension budget");
        let current = b.levels.last().context("System4 array builder stack")?;
        if current.values.len() == n {
            let page = b.levels.pop().unwrap();
            if let Some(parent) = b.levels.last_mut() {
                let slot = self.heap.alloc(Object::Page(Some(page)))?;
                parent.values.push(slot);
                self.actions.push(Action::Array(b));
            } else {
                self.heap.set_page(b.destination, Some(page))?;
                self.consume(b.dimensions.len() + 2)?;
                self.pc = b.next;
                self.running_actions = false;
            }
            return Ok(());
        }
        let pending: usize = b.levels.iter().map(|p| p.values.len()).sum();
        ensure!(
            self.heap
                .value_count()
                .checked_add(pending + 1)
                .is_some_and(|n| n <= heap::MAX_VALUES),
            "System4 pending array value budget"
        );
        if level + 1 < b.dimensions.len() {
            let rank = b.dimensions.len() - level - 1;
            b.levels.push(Page {
                kind: PageKind::Array {
                    kind: b.kind,
                    structure: b.structure,
                    rank,
                },
                values: Vec::new(),
            });
            self.actions.push(Action::Array(b));
            return Ok(());
        }
        let kind = element_kind(b.kind)?;
        if kind == 13 {
            let slot = self.alloc_structure(b.structure, 0)?;
            b.levels.last_mut().unwrap().values.push(slot);
            let index = b.structure;
            self.actions.push(Action::Array(b));
            self.actions.push(Action::InitStruct {
                index,
                slot,
                member: 0,
                depth: 0,
            });
        } else {
            let value = self.initial(kind)?;
            b.levels.last_mut().unwrap().values.push(value);
            self.actions.push(Action::Array(b));
        }
        Ok(())
    }
    fn array_numof(&self, slot: i32, rank: i32) -> Result<i32> {
        let Some(p) = self.heap.page_opt(slot)? else {
            return Ok(0);
        };
        let PageKind::Array { rank: r, .. } = p.kind else {
            bail!("System4 expected array")
        };
        if rank < 1 || rank as usize > r {
            return Ok(0);
        }
        if rank == 1 {
            return Ok(p.values.len() as i32);
        }
        self.array_numof(
            *p.values
                .first()
                .context("System4 empty multidimensional array access")?,
            rank - 1,
        )
    }
    fn copy_object(&mut self, slot: i32, depth: usize) -> Result<i32> {
        ensure!(depth < 64, "System4 copy nesting");
        let object = self.heap.slot(slot)?.object.clone();
        match object {
            Object::String(s) => self.heap.alloc(Object::String(s)),
            Object::Page(None) => self.heap.alloc(Object::Page(None)),
            Object::Page(Some(page)) => {
                // vm_copy_page reserves identity before recursive member copies.
                let target = self.heap.alloc(Object::Page(Some(Page {
                    kind: page.kind.clone(),
                    values: vec![0; page.values.len()],
                })))?;
                for (j, value) in page.values.into_iter().enumerate() {
                    let k = self.variable(slot, j as i32)?.kind;
                    let x = if matches!(k, 12 | 13 | 63) || is_array(k) {
                        self.copy_object(value, depth + 1)?
                    } else {
                        if is_reference(k) {
                            self.heap.reference(value)?;
                        }
                        value
                    };
                    self.heap.set(target, j as i32, x)?;
                }
                Ok(target)
            }
        }
    }
    /// Human-readable native location; strings remain byte data in the VM.
    pub fn location(&self) -> String {
        let frame = self.frames.last();
        format!(
            "pc={:#x} function={} instructions={} stack={} calls={} heap={} values={} main={}",
            self.pc,
            frame
                .map(
                    |f| String::from_utf8_lossy(&self.program.functions[f.function].name)
                        .into_owned()
                )
                .unwrap_or_default(),
            self.instructions,
            self.stack.len(),
            self.frames.len(),
            self.heap.live_slots(),
            self.heap.value_count(),
            self.entered_main
        )
    }
}
fn calculate(op: &str, x: i32, y: i32) -> Result<i32> {
    Ok(match op {
        "ADD" => x.wrapping_add(y),
        "SUB" => x.wrapping_sub(y),
        "MUL" => x.wrapping_mul(y),
        "DIV" => {
            ensure!(
                !(x == i32::MIN && y == -1),
                "System4 native signed division overflow"
            );
            if y == 0 { 0 } else { x.wrapping_div(y) }
        }
        "MOD" => {
            ensure!(
                !(x == i32::MIN && y == -1),
                "System4 native signed remainder overflow"
            );
            if y == 0 { 0 } else { x.wrapping_rem(y) }
        }
        "AND" => x & y,
        "OR" => x | y,
        "XOR" => x ^ y,
        "LSHIFT" => {
            ensure!((0..32).contains(&y), "System4 undefined native shift");
            x.wrapping_shl(y as u32)
        }
        "RSHIFT" => {
            ensure!((0..32).contains(&y), "System4 undefined native shift");
            x >> y
        }
        "LT" => (x < y) as i32,
        "GT" => (x > y) as i32,
        "LTE" => (x <= y) as i32,
        "GTE" => (x >= y) as i32,
        "NOTE" => (x != y) as i32,
        "EQUALE" => (x == y) as i32,
        k if k.starts_with("LI_") => {
            let (x, y) = (x as i64, y as i64);
            let n = match k {
                "LI_ADD" => x + y,
                "LI_SUB" => x - y,
                "LI_MUL" => x * y,
                "LI_DIV" => {
                    if y == 0 {
                        0
                    } else {
                        x / y
                    }
                }
                "LI_MOD" => {
                    ensure!(y != 0, "System4 LI_MOD zero divisor");
                    x % y
                }
                _ => unreachable!(),
            };
            n.clamp(0, i32::MAX as i64) as i32
        }
        k if k.starts_with("F_") => {
            let (x, y) = (f32::from_bits(x as u32), f32::from_bits(y as u32));
            match k {
                "F_ADD" => (x + y).to_bits() as i32,
                "F_SUB" => (x - y).to_bits() as i32,
                "F_MUL" => (x * y).to_bits() as i32,
                "F_DIV" => (x / y).to_bits() as i32,
                "F_LT" => (x < y) as i32,
                "F_GT" => (x > y) as i32,
                "F_LTE" => (x <= y) as i32,
                "F_GTE" => (x >= y) as i32,
                "F_NOTE" => (x != y) as i32,
                "F_EQUALE" => (x == y) as i32,
                _ => unreachable!(),
            }
        }
        _ => bail!("System4 unsupported arithmetic {op}"),
    })
}
