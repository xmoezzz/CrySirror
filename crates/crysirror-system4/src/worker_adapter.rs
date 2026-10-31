// Copyright (C) 2026 CrySirror contributors. GPL-2.0-or-later.
//! Owner/worker boundary for original VM execution. All file reads, AIN parsing,
//! heap/array allocation and instruction batches run on the shared task pool.
//! The owner alone completes typed host calls. No factory is registered here.
use crate::{
    adapter::{AbsentDebugLog, Bridge, Dispatch},
    ain::Program,
    vm::{Machine, Step},
};
use anyhow::{Result, ensure};
use crysirror_core::{
    Error,
    path::{Mount, VfsPath},
    service::{HostServices, InputFrame},
    state::ClientId,
    vm::{VmAdapter, VmYield},
};
use crysirror_tasks::{Task, TaskPool};
use std::sync::Arc;
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub program: String,
    /// Native logical view dimensions, read from original configuration.
    #[serde(default)]
    pub canvas: Option<[u32; 2]>,
    /// Explicit portable bindings for native Gothic/Mincho faces.
    #[serde(default = "font_aliases")]
    pub font_aliases: [String; 2],
    pub reviewed_output_log_dll: Option<String>,
}
fn font_aliases()->[String;2] {["default".into(),"default".into()]}
impl Config {
    pub fn validate(&self) -> Result<()> {
        if let Some(canvas)=self.canvas {crate::sact::Sact::new(canvas,self.font_aliases.clone())?;}

        for s in std::iter::once(&self.program).chain(self.reviewed_output_log_dll.iter()) {
            ensure!(
                VfsPath::new(Mount::Asset, s)?.relative() == s,
                "System4 canonical asset path"
            );
        }
        ensure!(
            self.program.to_ascii_lowercase().ends_with(".ain"),
            "System4 original AIN path"
        );
        Ok(())
    }
}
struct Completed {
    machine: Machine,
    stop: std::result::Result<Step, String>,
    debug_log: Option<AbsentDebugLog>,
}
pub struct NativeAdapter {
    client: ClientId,
    config: Config,
    pool: TaskPool,
    job: Option<Task<Completed>>,
    machine: Option<Machine>,
    bridge: Bridge,
    stop: Option<Step>,
    started: bool,
    last_location: String,
    last_error: Option<String>,
}
fn core_error(e: anyhow::Error) -> Error {
    Error::Io(format!("{e:#}"))
}
impl NativeAdapter {
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        let sact=config.canvas.map(|canvas|crate::sact::Sact::new(canvas,config.font_aliases.clone())).transpose()?;
        let mut bridge=Bridge::default();bridge.sact=sact;
        Ok(Self {
            client,
            config,
            pool: TaskPool::new("system4-native", 1, 2)?,
            job: None,
            machine: None,
            bridge,
            stop: None,
            started: false,
            last_location: String::new(),
            last_error: None,
        })
    }
    fn advance(&mut self, h: &mut dyn HostServices, budget: u32) -> Result<VmYield> {
        if let Some(error) = &self.last_error {
            anyhow::bail!("{error}")
        }
        if !self.started {
            let storage = h.storage(self.client)?;
            let path = VfsPath::new(Mount::Asset, &self.config.program)?;
            let dll = self.config.reviewed_output_log_dll.clone();
            self.job = Some(self.pool.spawn(move |cancel| {
                cancel.check()?;
                let reader = storage.open_asset(&path)?;
                let bytes = crysirror_resource::read_all(
                    reader.as_ref(),
                    crate::ain::MAX_AIN_BYTES as u64,
                )?;
                cancel.check()?;
                let p = Program::parse(&bytes)?;
                p.validate_code_layout()?;
                let profile = if let Some(dll) = dll {
                    let reader = storage.open_asset(&VfsPath::new(Mount::Asset, &dll)?)?;
                    let bytes = crysirror_resource::read_all(reader.as_ref(), 64 * 1024)?;
                    Some(AbsentDebugLog::verify_native_library(&bytes)?)
                } else {
                    None
                };
                cancel.check()?;
                let machine = Machine::new(Arc::new(p))?;
                Ok(Completed {
                    machine,
                    stop: Ok(Step::Advanced),
                    debug_log: profile,
                })
            })?);
            self.started = true;
        }
        if let Some(job) = &mut self.job {
            let Some(done) = job.try_take()? else {
                return Ok(VmYield::WaitUntil(h.now_ms().saturating_add(1)));
            };
            self.job = None;
            self.last_location = done.machine.location();
            self.machine = Some(done.machine);
            if done.debug_log.is_some() {
                self.bridge.debug_log = done.debug_log;
            }
            self.stop = Some(done.stop.map_err(anyhow::Error::msg)?);
        }
        match self.stop.clone() {
            Some(Step::Halted) => return Ok(VmYield::Halted),
            Some(Step::External(call)) => {
                match self
                    .bridge
                    .dispatch_for(self.client,self.machine.as_mut().unwrap(), &call, h)?
                {
                    Dispatch::WaitUntil(t) => return Ok(VmYield::WaitUntil(t)),
                    Dispatch::Completed => self.stop = None,
                }
            }
            _ => {}
        }
        if self.job.is_none() {
            // Reserve before transferring the only mutable native state to the worker.
            let permit = self.pool.reserve()?;
            let mut m = self
                .machine
                .take()
                .ok_or_else(|| anyhow::anyhow!("System4 missing worker state"))?;
            let count = budget.clamp(1, 100_000);
            self.job = Some(permit.spawn(move |cancel| {
                let mut stop = Ok(Step::Advanced);
                for n in 0..count {
                    if n % 256 == 0 {
                        cancel.check()?;
                    }
                    match m.step() {
                        Ok(Step::Advanced) => {}
                        Ok(s) => {
                            stop = Ok(s);
                            break;
                        }
                        Err(e) => {
                            stop = Err(format!("{e:#}"));
                            break;
                        }
                    }
                }
                Ok(Completed {
                    machine: m,
                    stop,
                    debug_log: None,
                })
            })?);
        }
        Ok(VmYield::BudgetExhausted)
    }
}
impl VmAdapter for NativeAdapter {
    fn engine_id(&self) -> &'static str {
        "system4"
    }
    fn snapshot_schema(&self) -> u32 {
        0
    }
    fn step(
        &mut self,
        h: &mut dyn HostServices,
        _input: InputFrame,
        budget: u32,
    ) -> crysirror_core::Result<VmYield> {
        let result = self.advance(h, budget);
        if let Err(e) = &result {
            self.last_error = Some(format!("{e:#}"));
        }
        result.map_err(core_error)
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported("System4 serialized native snapshot"))
    }
    fn restore(&mut self, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported("System4 serialized native snapshot"))
    }
    fn diagnostics(&self) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"engine":"system4","native_location":self.last_location,"error":self.last_error,"worker_pending":self.job.is_some(),"debug_log_interface":if self.bridge.debug_log.is_some(){"reviewed-absent"}else{"unreviewed"},"debug_log_calls":self.bridge.absent_debug_calls,"clock_calls":self.bridge.clock_calls,"log_calls":self.bridge.log_calls,"sact":self.bridge.sact.as_ref().map(|s|serde_json::json!({"initialized":s.initialized,"canvas":s.canvas,"font_ready":s.font_ready,"audio_ready":s.audio_ready,"calls":s.calls})),"factory_registered":false})).unwrap_or_default()
    }
}
