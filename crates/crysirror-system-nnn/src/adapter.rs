// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Core VM-port boundary: workers own original control/expressions; only the
//! engine owner submits scoped presentation/audio calls. Bootstrap resources
//! are prepared before construction; this is not a registered/playable factory.
use crate::{
    bridge::Bridge,
    machine::{Machine, NativeCall, Step},
    native_bootstrap::{NativeConfiguration, PreparedBoot},
    native_control::ScriptSession,
    Program,
};
use anyhow::{Context, Result};
use crysirror_core::{
    service::{HostServices, InputFrame},
    vm::{VmAdapter, VmYield},
    Error,
};
use crysirror_tasks::{Task, TaskPool};
use std::sync::Arc;

pub type ScriptLoader =
    Arc<dyn Fn(&[u8], &crysirror_tasks::Cancellation) -> Result<Arc<Program>> + Send + Sync>;
pub struct NativeAdapter {
    session: Option<ScriptSession>,
    loader: Option<ScriptLoader>,
    configuration: Option<Arc<NativeConfiguration>>,
    pending: Option<NativeCall>,
    worker: Option<Task<(ScriptSession, Result<Step>)>>,
    pool: TaskPool,
    bridge: Bridge,
    pc: u32,
    executed: u64,
    halted: bool,
    error: Option<String>,
    observed_script: Vec<u8>,
    observed_depth: usize,
    background_loop:Option<bool>,
    input_snapshot:InputFrame,
}
impl NativeAdapter {
    /// Resource loading/decryption/initial values must be prepared on the
    /// caller's scoped resource workers. No filesystem access occurs here.
    pub fn from_prepared(program: Arc<Program>, machine: Machine, bridge: Bridge) -> Result<Self> {
        Ok(Self {
            pc: machine.pc(),
            executed: machine.executed(),
            session: Some(ScriptSession::new(program, machine)),
            loader: None,
            configuration: None,
            pending: None,
            worker: None,
            pool: TaskPool::new("nnn-native-vm", 1, 1)?,
            bridge,
            halted: false,
            error: None,
            observed_script:b"sys".to_vec(),
            observed_depth:0,
            background_loop:None,
            input_snapshot:InputFrame::default(),
        })
    }
    /// Boot metadata and resource loading are prepared outside the owner loop.
    /// The loader is capability-scoped and called only by the admitted worker.
    pub fn from_bootstrap(
        boot: PreparedBoot,
        loader: ScriptLoader,
        mut bridge: Bridge,
    ) -> Result<Self> {
        let pc = boot.session.machine().pc();
        let executed = boot.session.executed();
        bridge.set_frame_time(u64::try_from(
            boot.setup.integer(b"frameTime")?.unwrap_or(50),
        )?)?;
        bridge.configure_functions(boot.functions.clone())?;
        let configuration = Some(Arc::new(boot.configuration()));
        let observed_script=boot.session.script_name().to_vec();let observed_depth=boot.session.depth();
        Ok(Self {
            session: Some(boot.session),
            loader: Some(loader),
            configuration,
            pending: None,
            worker: None,
            pool: TaskPool::new("nnn-native-vm", 1, 1)?,
            bridge,
            pc,
            executed,
            halted: false,
            error: None,
            observed_script,observed_depth,
            background_loop:None,
            input_snapshot:InputFrame::default(),
        })
    }
    /// Original outer Opening/Title modes run CBackExecClass on a separate
    /// script PC with the shared native variable bank and the same engine API
    /// bridge. The program is prepared by the capability loader, not copied.
    pub fn from_background(mut boot:PreparedBoot,loader:ScriptLoader,bridge:Bridge,name:Vec<u8>,program:Arc<Program>,mode:crate::native_shell::NativeMode,looping:bool)->Result<Self> {
        anyhow::ensure!(matches!(mode,crate::native_shell::NativeMode::Opening|crate::native_shell::NativeMode::Title),"NNN background native outer mode profile");
        anyhow::ensure!(boot.project.index(&name).is_some(),"NNN background program original membership");
        let mut variables=boot.session.machine().variables().context("NNN background variable bank absent")?.clone();
        anyhow::ensure!(variables.bind_calculated_value(b"mode",mode as i32)?,"NNN reviewed background mode calculated binding absent");
        boot.session=ScriptSession::background(name,program,Machine::with_variables(variables))?;
        let mut vm=Self::from_bootstrap(boot,loader,bridge)?;vm.background_loop=Some(looping);Ok(vm)
    }
    fn advance(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
        budget: u32,
        schedule_next:bool,
    ) -> Result<VmYield> {
        if schedule_next {self.input_snapshot=input;}
        if let Some(error) = &self.error {
            anyhow::bail!("System-NNN terminal error: {error}");
        }
        if self.halted {
            return Ok(VmYield::Halted);
        }
        if budget == 0 {
            return Ok(VmYield::BudgetExhausted);
        }
        if let Some(mut task) = self.worker.take() {
            match task.try_take()? {
                None => {
                    self.worker = Some(task);
                    return Ok(VmYield::BudgetExhausted);
                }
                Some((session, step)) => {
                    self.pc = session.machine().pc();
                    self.executed = session.executed();
                    self.observed_script=session.script_name().to_vec();self.observed_depth=session.depth();
                    self.session = Some(session);
                    match step? {
                        Step::Halted => {
                            self.halted = true;
                            return Ok(VmYield::Halted);
                        }
                        Step::Budget => {}
                        Step::Native(call) => {
                            self.pending = Some(call);
                        }
                    }
                }
            }
        }
        if let Some(call) = self.pending.as_ref() {
            let program = self.session.as_ref().unwrap().program().clone();
            if !self.bridge.try_commit_with_context(
                self.session.as_mut().unwrap().machine_mut(),
                call,
                &program,
                input,
                host,
            )? {
                return Ok(VmYield::BudgetExhausted);
            }
            self.pending = None;
            let session = self.session.as_ref().unwrap();
            self.pc = session.machine().pc();
            self.executed = session.executed();
        }
        if !schedule_next {return Ok(VmYield::BudgetExhausted)}
        // Reserve capacity before transferring VM state. The worker owns no
        // HostServices/window/GPU/audio pointer, and never joins its own pool.
        let permit = self.pool.reserve()?;
        let mut session = self
            .session
            .take()
            .context("System-NNN VM ownership absent")?;
        // GetCalcuVarData mouse/button fields are sampled engine input. Freeze
        // them at VM admission; workers do not query or mutate the host input.
        if let Some(current)=session.machine().variables() {
            let mut variables=current.clone();let input=self.input_snapshot;
            for (name,value) in [(b"mousex".as_slice(),input.pointer_x),(b"mousey",input.pointer_y),(b"trig1",i32::from(input.buttons_down&1!=0)),(b"trig2",i32::from(input.buttons_down&2!=0)),(b"trig3",i32::from(input.buttons_down&4!=0)),(b"click1",i32::from(input.buttons_pressed&1!=0)),(b"click2",i32::from(input.buttons_pressed&2!=0)),(b"click3",i32::from(input.buttons_pressed&4!=0))] {variables.bind_calculated_value(name,value)?;}
            session.machine_mut().replace_variables(variables)?;
        }
        let loader = self.loader.clone();
        let background_loop=self.background_loop;
        self.worker = Some(permit.spawn(move |cancel| {
            cancel.check()?;
            let result = (|| -> Result<Step> {
                // Each original instruction/control transfer spends admission
                // budget; even a cyclic controller yields to the owner.
                for _ in 0..budget.min(10000) {
                    cancel.check()?;
                    match session.poll(1)? {
                        Step::Native(call) => {
                            if let Some(looping)=background_loop {
                                if session.depth()==0&&matches!(call.payload,crate::machine::Payload::Control{code:0x55560006,..}) {
                                    session.acknowledge(&call)?;
                                    if looping {session.restart_background()?;continue;}
                                    return Ok(Step::Halted);
                                }
                            }
                            if let Some(load) = &loader {
                                if session.dispatch_control(&call, |name| load(name, &cancel))? {
                                    continue;
                                }
                            }
                            return Ok(Step::Native(call));
                        }
                        Step::Halted => {
                            anyhow::ensure!(background_loop.is_none(),"NNN background script reached END instead of original NEXT completion");
                            if loader.is_some() {
                                session.restart_controller_after_end()?;
                            } else {
                                return Ok(Step::Halted);
                            }
                        }
                        Step::Budget => {}
                    }
                }
                Ok(Step::Budget)
            })();
            cancel.check()?;
            Ok((session, result))
        })?);
        Ok(VmYield::BudgetExhausted)
    }
    /// Non-mutating readiness query for an offline host dependency fence.
    pub fn dependency_pending(&self)->bool {self.worker.is_some()||self.bridge.dependency_pending()}
    /// Poll only already-admitted work and commit its owner receipt. Never
    /// samples input, advances a clock, or schedules the next VM batch.
    pub fn poll_dependency(&mut self,host:&mut dyn HostServices)->crysirror_core::Result<()> {
        if !self.dependency_pending(){return Ok(())}
        match self.advance(host,InputFrame::default(),1024,false) {
            Ok(_)=>Ok(()),Err(error)=>{let text=format!("System-NNN original pc={:#x} instructions={}: {error:#}",self.pc,self.executed);self.error=Some(text.clone());Err(Error::Io(text))}
        }
    }
    pub fn bridge(&self) -> &Bridge {
        &self.bridge
    }
    /// Owner transition after original background NEXT. Does not serialize or
    /// reconstruct a native save and refuses a task/callback still in flight.
    pub fn into_idle_state(self)->Result<(Machine,Bridge)> {
        anyhow::ensure!(self.halted&&self.worker.is_none()&&self.pending.is_none(),"NNN outer transition requires completed original background NEXT");
        let session=self.session.as_ref().context("NNN completed background state absent")?;
        anyhow::ensure!(session.depth()==0,"NNN completed background has a subcontext");
        Ok((session.machine().clone(),self.bridge))
    }
    pub fn configuration(&self) -> Option<&Arc<NativeConfiguration>> {
        self.configuration.as_ref()
    }
}
impl VmAdapter for NativeAdapter {
    fn engine_id(&self) -> &'static str {
        "system-nnn"
    }
    fn snapshot_schema(&self) -> u32 {
        1
    }
    fn step(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
        budget: u32,
    ) -> crysirror_core::Result<VmYield> {
        match self.advance(host, input, budget,true) {
            Ok(result) => Ok(result),
            Err(error) => {
                let text = format!(
                    "System-NNN original pc={:#x} instructions={}: {error:#}",
                    self.pc, self.executed
                );
                self.error = Some(text.clone());
                Err(Error::Io(text))
            }
        }
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported(
            "System-NNN original resource/save reconstruction",
        ))
    }
    fn restore(&mut self, _data: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported(
            "System-NNN original resource/save reconstruction",
        ))
    }
    fn diagnostics(&self) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
        "vm":"original-system-nnn-spt","background_loop":self.background_loop,"native_location":format!("pc={:#x}",self.pc),
        "wait_kind":if self.worker.is_some(){"vm_worker"}else{self.bridge.wait_kind(self.pending.as_ref())},"dependency_pending":self.dependency_pending(),
        "instructions":self.executed,"worker_pending":self.worker.is_some(),"resource_pending":self.pending.is_some(),"script":String::from_utf8_lossy(&self.observed_script),"script_depth":self.observed_depth,"error":self.error,
        "engine_api_calls":self.bridge.api_calls,"overlap_captures":self.bridge.overlap_captures,
        "story_name_bytes":self.bridge.metadata.story_name.len(),"film_name_bytes":self.bridge.metadata.film_name.len(),
        "factory_registered":false,"owns_framebuffer":false,"halted":self.halted
    })).unwrap_or_default()
    }
}
