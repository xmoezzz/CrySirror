// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Independently written native instruction state. Numeric operation contracts
//! are researched format facts; no original engine implementation is included.
use crate::expression::{evaluate, Variables};
use crate::{Program, IDENTIFY_DATA};
use anyhow::{bail, ensure, Context, Result};
/// Original callback data, before an engine semantic adapter acknowledges it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    StoryName(Vec<u8>),
    FilmName(Vec<u8>),
    Image {
        preload: bool,
        slot: i32,
        name: Vec<u8>,
    },
    Message {
        kind: u32,
        serial: u32,
        cutin: i32,
        text: Vec<u8>,
    },
    /// Original bounded callback words. These have not been reinterpreted as
    /// shader/asset/clock arguments or claimed to be engine operations.
    Words {
        count: u32,
        words: Vec<i32>,
    },
    Scalar(i32),
    Empty,
    Control {
        code: u32,
        script: Option<Vec<u8>>,
        resume_id: Option<u32>,
    },
    UserCall {
        command: bool,
        parameters: Vec<i32>,
        resume_id: Option<u32>,
    },
    Voice {
        name: Option<Vec<u8>>,
        parameters: Vec<i32>,
        default_voice: i32,
    },
    Selection {
        choices: u32,
        messages: u32,
        string_ids: Vec<u32>,
        strings: Vec<Vec<u8>>,
        /// Original saved-selection lookup ID, separate from newer UI serial.
        resume_id: u32,
        time_limit: i32,
        auto_select: i32,
        special: i32,
        serial: u32,
    },
    DebugMessage(Vec<u8>),
    VariableWrites {
        writes: Vec<crate::expression::NativeVariableWrite>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeCall {
    pub serial: u64,
    pub pc: u32,
    pub next: u32,
    pub identify: u32,
    pub code: u32,
    pub payload: Payload,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Native(NativeCall),
    Budget,
    Halted,
}
#[derive(Clone, Debug)]
pub struct ReturnFrame {
    pub pc: u32,
    pub native_id: u32,
}
#[derive(Clone, Debug)]
pub struct Machine {
    pc: u32,
    executed: u64,
    next_serial: u64,
    pending: Option<NativeCall>,
    returns: Vec<ReturnFrame>,
    halted: bool,
    variables: Option<Variables>,
    pending_variables: Option<Variables>,
}
impl Default for Machine {
    fn default() -> Self {
        Self {
            pc: 0,
            executed: 0,
            next_serial: 1,
            pending: None,
            returns: Vec::new(),
            halted: false,
            variables: None,
            pending_variables: None,
        }
    }
}
impl Machine {
    pub fn with_variables(variables: Variables) -> Self {
        Self {
            variables: Some(variables),
            ..Self::default()
        }
    }
    pub fn variables(&self) -> Option<&Variables> {
        self.variables.as_ref()
    }
    /// Owner/worker transfer of the one native variable bank between script
    /// frames. A pending receipt owns its candidate and cannot be overwritten.
    pub fn replace_variables(&mut self, variables: Variables) -> Result<()> {
        ensure!(
            self.pending.is_none(),
            "System-NNN pending call owns variable candidate"
        );
        self.variables = Some(variables);
        Ok(())
    }
    pub fn restart(&mut self) -> Result<()> {
        ensure!(
            self.pending.is_none(),
            "System-NNN cannot restart pending instruction"
        );
        self.pc = 0;
        self.returns.clear();
        self.halted = false;
        Ok(())
    }
    pub fn return_frames(&self) -> &[ReturnFrame] {
        &self.returns
    }
    pub fn pc(&self) -> u32 {
        self.pc
    }
    pub fn executed(&self) -> u64 {
        self.executed
    }
    fn queue_native(
        &mut self,
        next: u32,
        identify: u32,
        code: u32,
        payload: Payload,
        variables: Option<Variables>,
    ) -> Result<Step> {
        let serial = self.next_serial;
        let next_serial = serial
            .checked_add(1)
            .context("System-NNN native receipt generation exhausted")?;
        let call = NativeCall {
            serial,
            pc: self.pc,
            next,
            identify,
            code,
            payload,
        };
        self.next_serial = next_serial;
        self.pending = Some(call.clone());
        self.pending_variables = variables;
        Ok(Step::Native(call))
    }
    /// Evaluate bounded original internal instructions. Native effects do not
    /// advance the PC until a matching owner-completed receipt is acknowledged.
    pub fn poll(&mut self, program: &Program, budget: u32) -> Result<Step> {
        ensure!(
            (1..=10000).contains(&budget),
            "System-NNN instruction budget"
        );
        if let Some(call) = &self.pending {
            return Ok(Step::Native(call.clone()));
        }
        if self.halted {
            return Ok(Step::Halted);
        }
        for _ in 0..budget {
            if self.pc as usize == program.word_count() {
                self.halted = true;
                return Ok(Step::Halted);
            }
            let i = program.instruction(self.pc)?;
            let operand = |n: usize| {
                i.operands
                    .get(n)
                    .copied()
                    .context("System-NNN missing instruction operand")
            };
            if i.identify == IDENTIFY_DATA {
                ensure!(
                    (0x55550001..=0x55550004).contains(&i.code),
                    "unknown System-NNN data record {:#x}",
                    i.code
                );
                self.pc = i.next;
                self.executed += 1;
                continue;
            }
            if i.identify == 0x66660005 {
                let mut vars = self
                    .variables
                    .clone()
                    .context("System-NNN variable profile not initialized")?;
                vars.begin_native_instruction();
                let (_, consumed) = evaluate(i.operands, program, &mut vars)?;
                ensure!(consumed == i.operands.len(), "SPT calculation operand span");
                if !vars.native_writes().is_empty() {
                    let payload = Payload::VariableWrites {
                        writes: vars.native_writes().to_vec(),
                    };
                    return self.queue_native(i.next, i.identify, i.code, payload, Some(vars));
                }
                vars.finish_native_instruction();
                self.variables = Some(vars);
                self.pc = i.next;
                self.executed += 1;
                continue;
            }
            if i.identify == 0x66660002
                && !matches!(
                    i.code,
                    0x55560006 | 0x5556000a..=0x5556000c | 0x55560015 | 0x55560016
                )
            {
                match i.code {
                    // The outer original YoyakuExecRoutine immediately resumes
                    // ENDIF/ENDKOMA without any callback or host operation.
                    0x55560001 | 0x5556000e | 0x55560013 => self.pc = i.next,
                    crate::control::OPEN | crate::control::CLOSE => self.pc = i.next,
                    0x55560002 | 0x55560011 => {
                        let mut vars = self
                            .variables
                            .clone()
                            .context("System-NNN variable profile not initialized")?;
                        vars.begin_native_instruction();
                        let next = crate::control::conditional(
                            program,
                            self.pc,
                            &mut vars,
                            i.code == 0x55560011,
                        )?;
                        if !vars.native_writes().is_empty() {
                            let payload = Payload::VariableWrites {
                                writes: vars.native_writes().to_vec(),
                            };
                            return self.queue_native(
                                next,
                                i.identify,
                                i.code,
                                payload,
                                Some(vars),
                            );
                        }
                        vars.finish_native_instruction();
                        self.variables = Some(vars);
                        self.pc = next;
                    }
                    0x55560003 | 0x55560004 | 0x55560008 => {
                        self.pc = crate::control::skip_body(program, i.next)?
                    }
                    0x55560005 => {
                        self.halted = true;
                        self.executed += 1;
                        return Ok(Step::Halted);
                    }
                    0x55560007 => {
                        let target = self
                            .returns
                            .last()
                            .context("System-NNN return without subroutine")?
                            .pc;
                        crate::control::target(program, target)?;
                        self.returns.pop();
                        self.pc = target;
                    }
                    0x55560009 => {
                        ensure!(
                            self.returns.len() < 249,
                            "System-NNN original call-depth limit"
                        );
                        let index = operand(0)? as usize;
                        ensure!(
                            index < program.words[8] as usize,
                            "System-NNN subroutine table index"
                        );
                        let table = *program
                            .words
                            .get(9)
                            .context("System-NNN missing sub table")?
                            as usize;
                        let target = *program
                            .words
                            .get(
                                table
                                    .checked_add(index)
                                    .context("System-NNN sub index overflow")?,
                            )
                            .context("System-NNN sub table bounds")?;
                        crate::control::target(program, target)?;
                        self.returns.push(ReturnFrame {
                            pc: i.next,
                            native_id: operand(1)?,
                        });
                        self.pc = target;
                    }
                    0x55560020 => {
                        let target = operand(0)?;
                        crate::control::target(program, target)?;
                        self.pc = target;
                    }
                    _ => bail!(
                        "unsupported System-NNN control pc={:#x} code={:#x}; PC retained",
                        self.pc,
                        i.code
                    ),
                }
                self.executed += 1;
                continue;
            }
            let mut candidate_variables = None;
            let payload=match (i.identify,i.code) {
                (0x66660002,code @ (0x55560006|0x5556000a..=0x5556000c|0x5556000e|0x55560013|0x55560015|0x55560016))=>Payload::Control {
                    code,script:if matches!(code,0x5556000a|0x55560015|0x55560016){Some(program.text(1,operand(0)?)?.to_vec())}else{None},
                    resume_id:if matches!(code,0x5556000a|0x55560016){Some(operand(1)?)}else{None},
                },
                (id @ (0x66660003|0x66660004),_)=>{
                    let count=operand(0)? as usize;ensure!(count<=256,"SPT user parameter count");
                    let mut vars=self.variables.clone().context("System-NNN variable profile not initialized")?;
                    vars.begin_native_instruction();
                    let mut at=1;let mut parameters=Vec::with_capacity(count);
                    for _ in 0..count {
                        let (value,used)=evaluate(i.operands.get(at..).context("SPT user parameter bounds")?,program,&mut vars)?;
                        at=at.checked_add(used).context("SPT parameter span overflow")?;parameters.push(value);
                    }
                    let resume_id=if id==0x66660003 {ensure!(at+1==i.operands.len(),"SPT command resume id/span");i.operands.last().copied()}else{ensure!(at==i.operands.len(),"SPT function operand span");None};
                    candidate_variables=Some(vars);
                    Payload::UserCall{command:id==0x66660003,parameters,resume_id}
                },
                (0x66660006,0x22220004)=>Payload::Words{count:1,words:vec![operand(1)? as i32]},
                (0x66660006,0x22220005)=>{
                    let count=operand(0)? as usize;
                    ensure!(count<=256&&count<=i.operands.len().saturating_sub(1),"SPT overlap parameter bounds");
                    Payload::Words{count:count as u32,words:i.operands[1..1+count].iter().map(|v|*v as i32).collect()}
                },
                (0x66660006,0x22220006)=>{
                    let choices=operand(0)? as usize;
                    ensure!((1..=250).contains(&choices),"SPT selection choice count");
                    let size=(i.next-i.pc) as usize;
                    let messages=if size==choices+5{0}else{*i.operands.last().context("SPT selection message count absent")? as usize};
                    ensure!(messages<=250&&choices+messages<=250,"SPT selection message budget");
                    let ids=i.operands.get(1..1+choices+messages).context("SPT selection string table bounds")?;
                    let strings=ids.iter().map(|id|program.text(1,*id).map(<[u8]>::to_vec)).collect::<Result<Vec<_>>>()?;
                    let mut time_limit=0;let mut auto_select=0;let mut special=0;let mut serial=0;
                    if size>choices+messages+5 {
                        time_limit=*i.operands.get(i.operands.len().checked_sub(2).context("SPT selection footer")?).context("SPT selection time")? as i32;
                        auto_select=*i.operands.get(i.operands.len().checked_sub(3).context("SPT selection footer")?).context("SPT selection auto")? as i32;
                    }
                    if size>choices+messages+6{special=*i.operands.get(i.operands.len().checked_sub(4).context("SPT selection footer")?).context("SPT selection special")? as i32;}
                    if size>choices+messages+7{serial=*i.operands.get(i.operands.len().checked_sub(5).context("SPT selection footer")?).context("SPT selection serial")?;}
                    let resume_id=operand(1+choices+messages)?;
                    Payload::Selection{choices:choices as u32,messages:messages as u32,string_ids:ids.to_vec(),strings,resume_id,time_limit,auto_select,special,serial}
                },
                (0x66660007,0x33330008)=>{
                    let count=operand(0)? as usize;ensure!((1..=256).contains(&count),"SPT voice parameter count");
                    let parameters=i.operands.get(1..1+count).context("SPT voice parameter bounds")?.iter().map(|v|*v as i32).collect::<Vec<_>>();
                    let name=if parameters[0]==-1{None}else{Some(program.text(1,parameters[0] as u32)?.to_vec())};
                    let default_voice=i.operands.get(1+count).copied().unwrap_or(0) as i32;
                    Payload::Voice{name,parameters,default_voice}
                },
                (0x66660007,0x33330042)=>Payload::DebugMessage(program.text(0,operand(0)?)?.to_vec()),
                (0x66660007,0x33330019|0x3333001a|0x33330028)=>Payload::Words{count:1,words:vec![operand(1)? as i32]},
                (0x66660007,0x3333001e)=>Payload::Scalar(operand(1)? as i32),
                (0x66660007,0x33330050)=>Payload::FilmName(program.text(1,operand(1)?)?.to_vec()),
                (0x66660007,0x33330051)=>Payload::StoryName(program.text(1,operand(1)?)?.to_vec()),
                (0x66660007,code @ (0x33330015|0x3333001f))=>Payload::Image{preload:code==0x3333001f,slot:operand(0)? as i32,name:program.text(1,operand(1)?)?.to_vec()},
                (0x66660006,kind @ 0x22220001..=0x22220003)=>Payload::Message{kind,serial:operand(1)?,cutin:i.operands.get(2).copied().unwrap_or(0) as i32,text:program.text(0,operand(0)?)?.to_vec()},
                (0x66660007,0x33330012|0x33330011|0x33330017|0x33330029)=>Payload::Empty,
                (0x66660007,0x33330052..=0x33330060)=>Payload::Scalar(operand(1)? as i32),
                (0x66660007,0x33330014|0x33330007|0x33330009|0x33330013|0x33330016|0x33330018|0x3333001b|0x3333001c|0x33330061..=0x3333006a|0x3333006c..=0x3333006f)=>{
                    let count=operand(0)?;ensure!(count<=256 && (count as usize)<=i.operands.len().saturating_sub(1),"System-NNN callback parameter bounds");
                    Payload::Words{count,words:i.operands[1..1+count as usize].iter().map(|v|*v as i32).collect()}
                }
                _=>bail!("unsupported System-NNN instruction pc={:#x} identify={:#x} code={:#x}; PC retained",self.pc,i.identify,i.code),
            };
            let next = if i.identify == 0x66660002 && i.code == 0x55560006 {
                self.pc
            } else {
                i.next
            };
            return self.queue_native(next, i.identify, i.code, payload, candidate_variables);
        }
        Ok(Step::Budget)
    }
    /// Validate before submitting an external owner operation, so stale input
    /// cannot mutate engine state and only fail at the later VM acknowledgement.
    pub fn validate_receipt(&self, receipt: &NativeCall) -> Result<()> {
        ensure!(
            self.pending.as_ref() == Some(receipt),
            "System-NNN stale/foreign native receipt"
        );
        Ok(())
    }
    pub fn acknowledge(&mut self, receipt: &NativeCall) -> Result<()> {
        self.validate_receipt(receipt)?;
        self.pc = receipt.next;
        if let Some(mut variables) = self.pending_variables.take() {
            variables.finish_native_instruction();
            self.variables = Some(variables);
        }
        self.executed += 1;
        self.pending = None;
        Ok(())
    }
    /// Prepare owner-side native state from the exact instruction candidate.
    /// Shared RNG/resource jobs update this detached bank before one receipt
    /// commits both their result and the original VM continuation.
    pub fn native_variables_candidate(&self, receipt: &NativeCall) -> Result<Variables> {
        self.validate_receipt(receipt)?;
        self.pending_variables
            .as_ref()
            .or(self.variables.as_ref())
            .cloned()
            .context("System-NNN native variable bank absent")
    }
    pub fn acknowledge_with_variables(
        &mut self,
        receipt: &NativeCall,
        variables: Variables,
    ) -> Result<()> {
        self.validate_receipt(receipt)?;
        self.pending_variables = Some(variables);
        self.acknowledge(receipt)
    }
    /// Native selection chooses a CASE after the callback record. Compute the
    /// destination before consuming its receipt so malformed tables retain PC.
    pub fn acknowledge_case(
        &mut self,
        receipt: &NativeCall,
        program: &Program,
        case: u16,
    ) -> Result<()> {
        self.validate_receipt(receipt)?;
        let target = crate::control::case_target(program, receipt.next, case)?;
        self.acknowledge(receipt)?;
        self.pc = target;
        Ok(())
    }
    /// Restore original script/command continuation IDs through their original
    /// table, independent of the VM's in-process return PC representation.
    pub fn resume_native_id(&mut self, program: &Program, id: u32, command: bool) -> Result<()> {
        ensure!(
            self.pending.is_none(),
            "System-NNN resume while native call pending"
        );
        let tables = program
            .call_tables
            .context("SPT continuation tables absent")?;
        let table = if command { tables[2] } else { tables[3] };
        let address = (table as usize)
            .checked_add(id as usize)
            .context("SPT continuation address overflow")?;
        let target = *program
            .words
            .get(address)
            .context("SPT continuation table bounds")?;
        let next = program.instruction(target)?.next;
        crate::control::target(program, next)?;
        self.pc = next;
        self.halted = false;
        Ok(())
    }
}
