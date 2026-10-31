// Copyright (c) 2026 CrySirror contributors.
// Native semantics researched against MajiroTools (C) 2021 Mario Welzig,
// Robert Jordan and its pinned opcode wiki. MIT; see SOURCE.json.
//! Bounded native stack execution. External calls remain unacknowledged at
//! their original PC until the host operation actually succeeds.
use crate::program::{Instruction, Operand, Program};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};
const MAX_STACK: usize = 65536;
const MAX_LOCALS: usize = 16384;
const MAX_FRAMES: usize = 128;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Integer(i32),
    Float(f32),
    String(Vec<u8>),
}
impl Value {
    pub fn integer(&self) -> Result<i32> {
        if let Self::Integer(n) = self {
            Ok(*n)
        } else {
            bail!("Majiro integer operand required")
        }
    }
    pub fn bytes(&self) -> Result<&[u8]> {
        if let Self::String(b) = self {
            Ok(b)
        } else {
            bail!("Majiro string operand required")
        }
    }
    fn default_type(t: u8) -> Result<Self> {
        match t {
            0 => Ok(Self::Integer(0)),
            1 => Ok(Self::Float(0.)),
            2 => Ok(Self::String(vec![])),
            _ => bail!("Majiro array allocation requires native reference semantics"),
        }
    }
    fn kind(&self) -> u8 {
        match self {
            Self::Integer(_) => 0,
            Self::Float(_) => 1,
            Self::String(_) => 2,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Arguments are in native parameter order (stack top first).
    Native {
        pc: u32,
        hash: u32,
        arguments: Vec<Value>,
        returns: bool,
    },
    Text {
        pc: u32,
        bytes: Vec<u8>,
    },
    ProcessText {
        pc: u32,
    },
    Control {
        pc: u32,
        bytes: Vec<u8>,
    },
    Halted,
}
#[derive(Clone)]
struct Frame {
    return_pc: u32,
    stack_base: usize,
    arguments: Vec<Value>,
    locals: Vec<Value>,
    discard_return: bool,
}
pub struct Machine {
    pub program: Arc<Program>,
    pub pc: u32,
    pub line: u16,
    pub executed: u64,
    pub stack: Vec<Value>,
    frames: Vec<Frame>,
    banks: [BTreeMap<u32, Value>; 3],
    pending: Option<(Event, u32, usize)>,
    halted: bool,
}
impl Machine {
    pub fn new(program: Arc<Program>) -> Self {
        Self {
            pc: program.entry,
            program,
            line: 0,
            executed: 0,
            stack: vec![],
            frames: vec![Frame {
                return_pc: 0,
                stack_base: 0,
                arguments: vec![],
                locals: vec![],
                discard_return: false,
            }],
            banks: std::array::from_fn(|_| BTreeMap::new()),
            pending: None,
            halted: false,
        }
    }
    pub fn pending(&self) -> Option<&Event> {
        self.pending.as_ref().map(|p| &p.0)
    }
    pub fn acknowledge(&mut self, outputs: Vec<Value>) -> Result<()> {
        let (event, next, consumed) = self
            .pending
            .as_ref()
            .context("no pending Majiro operation")?;
        let expected = matches!(event, Event::Native { returns: true, .. }) as usize;
        ensure!(outputs.len() == expected, "Majiro native return count");
        let keep = self
            .stack
            .len()
            .checked_sub(*consumed)
            .context("Majiro native stack receipt")?;
        ensure!(
            keep + outputs.len() <= MAX_STACK,
            "Majiro return stack budget"
        );
        let next = *next;
        self.stack.truncate(keep);
        self.stack.extend(outputs);
        self.pc = next;
        self.executed += 1;
        self.pending = None;
        Ok(())
    }
    fn queue(&mut self, event: Event, next: u32, consumed: usize) -> Event {
        self.pending = Some((event.clone(), next, consumed));
        event
    }
    fn tail(&self, n: usize) -> Result<&[Value]> {
        let start = self
            .stack
            .len()
            .checked_sub(n)
            .context("Majiro stack underflow")?;
        ensure!(
            start >= self.frames.last().unwrap().stack_base,
            "Majiro operand crosses call frame"
        );
        Ok(&self.stack[start..])
    }
    fn push(&mut self, v: Value) -> Result<()> {
        ensure!(self.stack.len() < MAX_STACK, "Majiro stack budget");
        self.stack.push(v);
        Ok(())
    }
    fn local_index(&self, offset: i16) -> Result<usize> {
        ensure!(offset >= 0, "Majiro destination local offset");
        let n = offset as usize;
        ensure!(
            n < self.frames.last().unwrap().locals.len(),
            "Majiro local outside allocation"
        );
        Ok(n)
    }
    fn variable(&self, flags: u16, hash: u32, offset: i16) -> Result<Value> {
        let scope = (flags >> 5) & 7;
        let ty = ((flags >> 8) & 7) as u8;
        ensure!(
            flags & 0xe000 == 0 && ty <= 2,
            "unsupported Majiro variable flags/type"
        );
        if scope == 3 {
            let f = self.frames.last().unwrap();
            if offset == -1 {
                return Ok(Value::Integer(f.arguments.len() as i32));
            }
            let v = if offset >= 0 {
                f.locals.get(offset as usize)
            } else {
                let n = (-2i32 - offset as i32) as usize;
                f.arguments
                    .len()
                    .checked_sub(n + 1)
                    .and_then(|i| f.arguments.get(i))
            }
            .context("Majiro local/argument offset")?;
            ensure!(v.kind() == ty, "Majiro variable type mismatch");
            Ok(v.clone())
        } else {
            ensure!(scope < 3, "Majiro scope");
            let v = self.banks[scope as usize]
                .get(&hash)
                .cloned()
                .unwrap_or(Value::default_type(ty)?);
            ensure!(v.kind() == ty, "Majiro variable type mismatch");
            Ok(v)
        }
    }
    fn store(&mut self, flags: u16, hash: u32, offset: i16, value: Value) -> Result<()> {
        let scope = (flags >> 5) & 7;
        ensure!(scope <= 3, "Majiro scope");
        if scope == 3 {
            let i = self.local_index(offset)?;
            ensure!(
                self.frames.last().unwrap().locals[i].kind() == value.kind(),
                "Majiro store type mismatch"
            );
            self.frames.last_mut().unwrap().locals[i] = value;
        } else {
            ensure!(
                self.banks[scope as usize].len() < 65536
                    || self.banks[scope as usize].contains_key(&hash),
                "Majiro variable budget"
            );
            self.banks[scope as usize].insert(hash, value);
        }
        Ok(())
    }
    pub fn step(&mut self) -> Result<Option<Event>> {
        if let Some(event) = self.pending() {
            return Ok(Some(event.clone()));
        }
        if self.halted {
            return Ok(Some(Event::Halted));
        }
        let i = self.program.instruction(self.pc)?.clone();
        self.execute(&i).with_context(|| {
            format!(
                "Majiro pc={:#x} line={} opcode={:#x}",
                self.pc, self.line, i.opcode
            )
        })
    }
    fn execute(&mut self, i: &Instruction) -> Result<Option<Event>> {
        let mut next = i.next;
        match (&i.opcode,&i.operand) {
   (0x800,Operand::Integer(n))=>self.push(Value::Integer(*n))?,
   (0x801,Operand::String(b))=>self.push(Value::String(b.clone()))?,
   (0x803,Operand::Float(n))=>self.push(Value::Float(*n))?,
   (0x83a,Operand::Line(n))=>self.line=*n,
   (0x829,Operand::Types(types))=>{ensure!(self.frames.last().unwrap().locals.len()+types.len()<=MAX_LOCALS,"Majiro local budget");let v=types.iter().map(|t|Value::default_type(*t)).collect::<Result<Vec<_>>>()?;self.frames.last_mut().unwrap().locals.extend(v);},
   (0x836,Operand::Types(types))=>{let a=&self.frames.last().unwrap().arguments;ensure!(types.len()==a.len() && types.iter().zip(a.iter().rev()).all(|(t,v)|*t==v.kind()),"Majiro function argument signature");},
   (0x802,Operand::Variable{flags,hash,offset})=>{
    let mut value=self.variable(*flags,*hash,*offset)?;let modify=flags&7;let invert=(flags>>3)&3;
    ensure!(self.stack.len()<MAX_STACK,"Majiro stack budget");
    let changed=if modify!=0{let n=value.integer()?;let updated=match modify{1|3=>n.wrapping_add(1),2|4=>n.wrapping_sub(1),_=>bail!("Majiro variable modifier")};if modify<=2{value=Value::Integer(updated);}Some(Value::Integer(updated))}else{None};
    if invert!=0{value=match value{Value::Integer(n)=>Value::Integer(match invert{1=>n.wrapping_neg(),2=>i32::from(n==0),_=>!n}),Value::Float(n) if invert==1=>Value::Float(-n),_=>bail!("Majiro invert type")};}
    if let Some(changed)=changed{self.store(*flags,*hash,*offset,changed)?;}self.stack.push(value);
   },
   (op,Operand::Variable{flags,hash,offset}) if (0x1b0..=0x265).contains(op)=>{
    let base=if *op>=0x210{0x210}else{0x1b0};let group=(*op-base)/8;let ty=((*op-base)%8) as u8;
    let rhs=self.tail(1)?[0].clone();ensure!(rhs.kind()==ty,"Majiro assignment type");
    let value=if group==0{rhs}else{let old=self.variable(*flags,*hash,*offset)?;integer_binary(match group{1=>0x100,2=>0x108,3=>0x110,4=>0x118,5=>0x120,6=>0x130,7=>0x128,8=>0x180,9=>0x168,10=>0x188,_=>bail!("Majiro assignment group")},&old,&rhs)?};
    self.store(*flags,*hash,*offset,value.clone())?;self.stack.pop();if base==0x1b0{self.stack.push(value);}
   },
   (op,Operand::None) if *op>=0x100 && *op<=0x188=>{
    let a=self.tail(2)?;let value=integer_binary(*op,&a[0],&a[1])?;self.stack.truncate(self.stack.len()-2);self.stack.push(value);
   },
   (0x190|0x198|0x1a0,Operand::None)=>{let n=self.tail(1)?[0].integer()?;let value=match i.opcode{0x190=>i32::from(n==0),0x198=>!n,_=>n.wrapping_neg()};*self.stack.last_mut().unwrap()=Value::Integer(value);},
   (0x191|0x1a8|0x1a9,Operand::None)=>{},
   (0x82f,Operand::None)=>{self.tail(1)?;self.stack.pop();},
   (0x82c,Operand::Jump(target))=>next=*target,
   (0x82d|0x82e,Operand::Jump(target))=>{let v=self.tail(1)?[0].integer()?;if (v!=0)==(i.opcode==0x82d){next=*target;}self.stack.pop();},
   (0x80f|0x810,Operand::Call{hash,arguments})=>{
    ensure!(self.frames.len()<MAX_FRAMES,"Majiro call depth budget");let target=*self.program.functions.get(hash).context("Majiro external library function is not loaded")?;
    let args=self.tail(*arguments as usize)?.to_vec();let base=self.stack.len()-args.len();
    self.frames.push(Frame{return_pc:i.next,stack_base:base,arguments:args,locals:vec![],discard_return:i.opcode==0x810});self.stack.truncate(base);next=target;
   },
   (0x82b,Operand::None)=>{
    let value=self.tail(1)?[0].clone();let f=self.frames.last().unwrap();
    ensure!(self.stack.len()==f.stack_base+1,"Majiro return operand shape");
    let f=self.frames.pop().unwrap();self.stack.truncate(f.stack_base);
    if self.frames.is_empty(){self.stack.push(value);self.halted=true;}else{if !f.discard_return{self.stack.push(value);}next=f.return_pc;}
   },
   (0x834|0x835,Operand::Call{hash,arguments})=>{let args=self.tail(*arguments as usize)?.iter().rev().cloned().collect();let event=Event::Native{pc:self.pc,hash:*hash,arguments:args,returns:i.opcode==0x834};return Ok(Some(self.queue(event,i.next,*arguments as usize)));},
   (0x840,Operand::String(bytes))=>return Ok(Some(self.queue(Event::Text{pc:self.pc,bytes:bytes.clone()},i.next,0))),
   (0x841,Operand::None)=>return Ok(Some(self.queue(Event::ProcessText{pc:self.pc},i.next,0))),
   (0x842,Operand::String(bytes))=>return Ok(Some(self.queue(Event::Control{pc:self.pc,bytes:bytes.clone()},i.next,0))),
   _=>bail!("unsupported original Majiro instruction {:#x}; execution stopped without advancing",i.opcode),
  }
        self.pc = next;
        self.executed += 1;
        Ok(None)
    }
}
fn integer_binary(op: u16, a: &Value, b: &Value) -> Result<Value> {
    ensure!(
        op & 7 == 0,
        "Majiro noninteger arithmetic profile unsupported"
    );
    let a = a.integer()?;
    let b = b.integer()?;
    let result = match op {
        0x100 => a.wrapping_mul(b),
        0x108 => a
            .checked_div(b)
            .context("Majiro exceptional integer division needs verified native profile")?,
        0x110 => a
            .checked_rem(b)
            .context("Majiro exceptional integer remainder needs verified native profile")?,
        0x118 => a.wrapping_add(b),
        0x120 => a.wrapping_sub(b),
        0x128 => a.wrapping_shr(b as u32),
        0x130 => a.wrapping_shl(b as u32),
        0x138 => i32::from(a <= b),
        0x140 => i32::from(a < b),
        0x148 => i32::from(a >= b),
        0x150 => i32::from(a > b),
        0x158 => i32::from(a == b),
        0x160 => i32::from(a != b),
        0x168 => a ^ b,
        0x170 => i32::from(a != 0 && b != 0),
        0x178 => i32::from(a != 0 || b != 0),
        0x180 => a & b,
        0x188 => a | b,
        _ => bail!("Majiro integer operator"),
    };
    Ok(Value::Integer(result))
}
