// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Original four execution contexts. This is VM policy and contains no host
//! service access, rendering, resource decoding or process-global filesystem.
use crate::{
    machine::{Machine, NativeCall, Payload, Step},
    Program,
};
use anyhow::{bail, ensure, Context, Result};
use std::sync::Arc;

#[derive(Clone, Debug)]
struct Frame {
    name: Vec<u8>,
    program: Arc<Program>,
    machine: Machine,
}
#[derive(Clone, Debug)]
pub struct ScriptSession {
    frames: Vec<Frame>,
    retired_instructions: u64,
}
impl ScriptSession {
    /// Context zero is the original sys.spt controller, including for NewGame.
    pub fn new(controller: Arc<Program>, machine: Machine) -> Self {
        Self {
            frames: vec![Frame {
                name: b"sys".to_vec(),
                program: controller,
                machine,
            }],
            retired_instructions: 0,
        }
    }
    pub fn background(name:Vec<u8>,program:Arc<Program>,machine:Machine)->Result<Self> {
        ensure!(!name.is_empty()&&name.len()<=128&&!name.contains(&0),"NNN background original script name bounds");
        Ok(Self{frames:vec![Frame{name,program,machine}],retired_instructions:0})
    }
    /// Original CBackExecClass::ReStart resets its own PC and return stack;
    /// it does not restart sys or clear shared game variables.
    pub fn restart_background(&mut self)->Result<()> {
        ensure!(self.frames.len()==1,"NNN background restart with active subcontext");
        self.frames[0].machine.restart()
    }
    pub fn machine(&self) -> &Machine {
        &self.frames.last().unwrap().machine
    }
    pub fn machine_mut(&mut self) -> &mut Machine {
        &mut self.frames.last_mut().unwrap().machine
    }
    pub fn program(&self) -> &Arc<Program> {
        &self.frames.last().unwrap().program
    }
    pub fn script_name(&self) -> &[u8] {
        &self.frames.last().unwrap().name
    }
    pub fn depth(&self) -> usize {
        self.frames.len() - 1
    }
    pub fn executed(&self) -> u64 {
        self.retired_instructions
            + self
                .frames
                .iter()
                .map(|f| f.machine.executed())
                .sum::<u64>()
    }
    pub fn poll(&mut self, budget: u32) -> Result<Step> {
        let frame = self.frames.last_mut().unwrap();
        frame.machine.poll(&frame.program, budget)
    }
    pub fn acknowledge(&mut self, receipt: &NativeCall) -> Result<()> {
        self.machine_mut().acknowledge(receipt)
    }
    /// Called on the admitted VM/resource worker. The loader must use a scoped
    /// reader. Loading/parsing finishes before the old PC or frames change.
    /// Returns false for callbacks requiring an owner/semantic implementation.
    pub fn dispatch_control(
        &mut self,
        receipt: &NativeCall,
        mut load: impl FnMut(&[u8]) -> Result<Arc<Program>>,
    ) -> Result<bool> {
        self.machine().validate_receipt(receipt)?;
        let Payload::Control { code, script, .. } = &receipt.payload else {
            return Ok(false);
        };
        match *code {
            0x5556000a | 0x55560015 | 0x55560016 => {
                let name = script.as_ref().context("SPT script name absent")?;
                let jump = *code == 0x55560015;
                ensure!(
                    jump || self.frames.len() < 4,
                    "System-NNN original script context depth exceeded"
                );
                let program = load(name)?;
                program.instruction(0)?;
                let mut old = self.frames.last().unwrap().clone();
                old.machine.acknowledge(receipt)?;
                let variables = old
                    .machine
                    .variables()
                    .context("System-NNN script variable bank absent")?
                    .clone();
                let next = Frame {
                    name: name.clone(),
                    program,
                    machine: Machine::with_variables(variables),
                };
                if jump {
                    self.retired_instructions = self
                        .retired_instructions
                        .checked_add(old.machine.executed())
                        .context("System-NNN instruction total overflow")?;
                    *self.frames.last_mut().unwrap() = next;
                } else {
                    *self.frames.last_mut().unwrap() = old;
                    self.frames.push(next);
                }
                Ok(true)
            }
            0x55560006 => {
                ensure!(
                    self.frames.len() > 1,
                    "System-NNN NEXT without caller context; PC retained"
                );
                let mut completed = self.frames.last().unwrap().machine.clone();
                completed.acknowledge(receipt)?;
                let variables = completed
                    .variables()
                    .context("System-NNN script variable bank absent")?
                    .clone();
                let parent = self.frames.len() - 2;
                self.frames[parent].machine.replace_variables(variables)?;
                self.retired_instructions = self
                    .retired_instructions
                    .checked_add(completed.executed())
                    .context("System-NNN instruction total overflow")?;
                self.frames.pop();
                Ok(true)
            }
            // ENDFILM is semantically resumable only outside scene replay;
            // callers retain it for their native read/film policy.
            _ => Ok(false),
        }
    }
    /// END returns to context zero and restarts sys, as in YoyakuExecRoutine.
    /// The caller invokes this on Step::Halted; an uncompleted callback cannot
    /// be discarded under the guise of reaching END.
    pub fn restart_controller_after_end(&mut self) -> Result<()> {
        let mut active = self.frames.last().unwrap().machine.clone();
        ensure!(
            matches!(active.poll(self.program(), 1)?, Step::Halted),
            "System-NNN controller reset requires END"
        );
        let variables = active
            .variables()
            .context("System-NNN script variable bank absent")?
            .clone();
        let mut controller = self.frames[0].clone();
        controller.machine.replace_variables(variables)?;
        controller.machine.restart()?;
        self.retired_instructions = self
            .retired_instructions
            .checked_add(
                self.frames
                    .iter()
                    .skip(1)
                    .map(|f| f.machine.executed())
                    .sum::<u64>(),
            )
            .context("System-NNN instruction total overflow")?;
        self.frames = vec![controller];
        Ok(())
    }
    pub fn complete_case(&mut self, receipt: &NativeCall, case: u16) -> Result<()> {
        let frame = self.frames.last_mut().unwrap();
        frame
            .machine
            .acknowledge_case(receipt, &frame.program, case)
    }
    pub fn user_name<'a>(
        &self,
        receipt: &NativeCall,
        commands: &'a crate::config::Names,
        functions: &'a crate::config::Names,
    ) -> Result<&'a [u8]> {
        self.machine().validate_receipt(receipt)?;
        let Payload::UserCall { command, .. } = receipt.payload else {
            bail!("System-NNN expected user callback")
        };
        let table = if command { commands } else { functions };
        let index = (receipt.code as usize)
            .checked_mul(2)
            .context("System-NNN user callback index overflow")?;
        table
            .tokens
            .get(index)
            .map(Vec::as_slice)
            .context("System-NNN user callback name index")
    }
}
