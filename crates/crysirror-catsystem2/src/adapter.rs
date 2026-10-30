// SPDX-License-Identifier: MPL-2.0
use crate::kcs::{Event, Machine, Program};
use anyhow::{ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{EngineQuery, HostServices, InputFrame},
    state::ClientId,
    vm::{VmAdapter, VmYield},
    Error,
};
use crysirror_tasks::{Task, TaskPool};
use serde::{Deserialize, Serialize};
use serde_json::json;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Capability-relative original KCS entry; no substituted CST boot script.
    pub script: String,
    /// Licensed font alias belonging to this engine project.
    pub font: String,
    /// Original startup.xml ETC/heap byte count.
    #[serde(default = "heap")]
    pub heap_bytes: u32,
    /// Caller-owned PE module read only as data to verify native syscall ABI.
    #[serde(default)]
    pub executable: Option<String>,
    /// Original virtual canvas from startup.xml, independent of host monitor.
    #[serde(default = "canvas")]
    pub canvas: [u32; 2],
}
fn canvas() -> [u32; 2] {
    [1280, 720]
}
fn heap() -> u32 {
    1024 * 1024
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            VfsPath::new(Mount::Asset, &self.script)?.relative() == self.script
                && self.script.to_ascii_lowercase().ends_with(".kcs"),
            "CatSystem2 original KCS entry path"
        );
        ensure!(
            !self.font.is_empty() && self.font.len() <= 256,
            "CatSystem2 font alias"
        );
        ensure!(
            (4..=16 * 1024 * 1024).contains(&self.heap_bytes),
            "CatSystem2 heap budget"
        );
        if let Some(path) = &self.executable {
            ensure!(
                VfsPath::new(Mount::Asset, path)?.relative() == path,
                "CatSystem2 module capability path"
            );
        }
        ensure!(
            self.canvas.iter().all(|n| (1..=8192).contains(n)),
            "CatSystem2 canvas budget"
        );
        Ok(())
    }
}
pub struct CatSystem2Vm {
    client: ClientId,
    config: Config,
    pool: Option<TaskPool>,
    loading: Option<Task<Prepared>>,
    program: Option<Program>,
    machine: Option<Machine>,
    error: Option<String>,
    native_abi: bool,
    debug_key_absent: bool,
    menu_installed: bool,
    tasks: crate::task_policy::NativeTaskPolicy,
}
struct Prepared {
    program: Program,
    menu: Option<crysirror_scene::ui::menu::MenuSpec>,
    native_abi: bool,
    debug_key_absent: bool,
}
impl CatSystem2Vm {
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            client,
            config,
            pool: None,
            loading: None,
            program: None,
            machine: None,
            error: None,
            native_abi: false,
            debug_key_absent: false,
            menu_installed: false,
            tasks: crate::task_policy::NativeTaskPolicy::bootstrap()?,
        })
    }
    fn run(&mut self, host: &mut dyn HostServices, budget: u32) -> Result<VmYield> {
        if self.program.is_none() {
            if self.loading.is_none() {
                let storage = host.storage(self.client)?;
                let path = VfsPath::new(Mount::Asset, &self.config.script)?;
                let config = self.config.clone();
                let pool = TaskPool::new("catsystem2-source", 1, 1)?;
                self.loading = Some(pool.spawn(move |cancel| {
                    cancel.check()?;
                    let reader = storage.open_asset(&path)?;
                    let bytes = crysirror_resource::read_all(
                        reader.as_ref(),
                        crate::kcs::MAX_SCRIPT_BYTES as u64,
                    )?;
                    cancel.check()?;
                    let program=Program::parse(&bytes)?;
                    let mut menu=None;
                    let mut native_abi=false;
                    let mut debug_key_absent=false;
                    if let Some(module)=config.executable.as_ref() {
                        let module=storage.open_asset(&VfsPath::new(Mount::Asset,module)?)?;
                        let module=crysirror_resource::read_all(module.as_ref(),crysirror_resource::pe::MAX_PE_BYTES as u64)?;
                        cancel.check()?;
                        let pe=crysirror_resource::pe::PeData::parse(&module)?;
                        // This is a verified native ABI fingerprint, never a game
                        // script/PC shortcut. Generic KCS parsing remains unpinned.
                        native_abi=pe.sha256()==[0xf8,0xa1,0xfb,0x4f,0xed,0x8a,0x0c,0x64,0xc3,0xfb,0xc0,0xbc,0xf0,0x09,0xf6,0x3e,0x8f,0x31,0x81,0xcf,0xbc,0x1a,0x0d,0xbd,0x60,0xf9,0xdd,0xef,0xc8,0xe2,0x5b,0x3d];
                        if native_abi {
                            debug_key_absent=storage.metadata(Mount::Asset,"cs2_debug_key.dat")?.is_none();
                            ensure!(debug_key_absent,"CatSystem2 present debug key requires unsupported native validation");
                            let resources=pe.menu_templates()?;
                            menu=Some(crate::native_menu::build(crate::native_menu::production(&resources)?,config.canvas,&config.font)?);
                        }
                    }
                    Ok(Prepared{program,menu,native_abi,debug_key_absent})
                })?);
                self.pool = Some(pool);
                return Ok(VmYield::BudgetExhausted);
            }
            let Some(prepared) = self.loading.as_mut().unwrap().try_take()? else {
                return Ok(VmYield::BudgetExhausted);
            };
            if let Some(menu) = prepared.menu {
                host.submit_engine(
                    self.client,
                    1,
                    &serde_json::to_vec(&json!({"ui":[{"op":"set_menu","menu":menu}]}))?,
                )?;
                self.menu_installed = true;
            }
            self.native_abi = prepared.native_abi;
            self.debug_key_absent = prepared.debug_key_absent;
            self.machine = Some(Machine::new(&prepared.program, self.config.heap_bytes)?);
            self.program = Some(prepared.program);
            self.loading = None;
        }
        if self.menu_installed {
            let events: Vec<crysirror_scene::ui::UiEvent> =
                serde_json::from_slice(&host.query_engine(self.client, EngineQuery::MenuEvents)?)?;
            for event in events {
                if event.menu_bar == Some(crate::native_menu::PRODUCTION_MENU) {
                    anyhow::bail!(
                        "unsupported CatSystem2 native menu WM_COMMAND {}",
                        event.widget.id
                    );
                }
            }
        }
        let machine = self
            .machine
            .as_mut()
            .context("CatSystem2 machine missing")?;
        let mut remaining = budget;
        loop {
            let before = machine.instructions;
            let event = machine.step(self.program.as_ref().unwrap(), remaining)?;
            remaining = remaining
                .saturating_sub((machine.instructions - before).min(u64::from(u32::MAX)) as u32);
            match event {
                Event::Halted => return Ok(VmYield::Halted),
                Event::BudgetExhausted => return Ok(VmYield::BudgetExhausted),
                Event::Native(call) => {
                    ensure!(
                        self.native_abi,
                        "CatSystem2 native gcall {} has no reviewed module ABI at PC {:#x}",
                        call.id,
                        call.pc
                    );
                    match call.id {
                        68 => {
                            ensure!(call.arguments.is_empty(),"CatSystem2 current-task query arguments");
                            machine.acknowledge(Some(self.tasks.current()))?;
                        }
                        8 => {
                            ensure!(call.arguments.len()==2,"CatSystem2 virtual screen-size arguments");
                            // The reviewed native draw interface returns its
                            // configured virtual width/height, not monitor size.
                            machine.commit_native(None,&[
                                (call.arguments[0],self.config.canvas[0].to_le_bytes().to_vec()),
                                (call.arguments[1],self.config.canvas[1].to_le_bytes().to_vec())
                            ])?;
                        }
                        13 => {
                            ensure!(call.arguments.len()==2,"CatSystem2 native string-copy arguments");
                            let mut bytes=machine.read_cstring(call.arguments[1],65534)?;
                            let destination=u64::from(call.arguments[0]);
                            let source=u64::from(call.arguments[1]);
                            let overlaps=destination<source+bytes.len() as u64+1&&source<destination+bytes.len() as u64+2;
                            ensure!(!overlaps||destination==source,"unsupported overlapping native CatSystem2 string copy");
                            // Original sub655C10 terminates with a WORD zero.
                            bytes.extend([0,0]);
                            machine.commit_native(Some(call.arguments[0]),&[(call.arguments[0],bytes)])?;
                        }
                        393=>{
                            ensure!(call.arguments.len()==1&&self.menu_installed,"CatSystem2 GMenuReturnTitle argument/menu boundary");
                            host.submit_engine(self.client,1,&serde_json::to_vec(&json!({"ui":[{"op":"set_menu_enabled","id":crate::native_menu::PRODUCTION_MENU,"item":crate::native_menu::RETURN_TITLE,"enabled":call.arguments[0]!=0}]}))?)?;
                            machine.acknowledge(None)?;
                        }
                        465=>{
                            ensure!(call.arguments.is_empty()&&self.debug_key_absent,"CatSystem2 debug query requires verified absent native debug key");
                            machine.acknowledge(Some(0))?;
                        }
                        _=>anyhow::bail!("CatSystem2 unsupported native gcall {} at {} PC {:#x} ({} argument bytes); original call remains unacknowledged",call.id,self.config.script,call.pc,call.arguments.len()*4),
                    }
                    if remaining == 0 {
                        return Ok(VmYield::BudgetExhausted);
                    }
                }
            }
        }
    }
}
impl VmAdapter for CatSystem2Vm {
    fn engine_id(&self) -> &'static str {
        "catsystem2"
    }
    fn snapshot_schema(&self) -> u32 {
        1
    }
    fn step(
        &mut self,
        host: &mut dyn HostServices,
        _input: InputFrame,
        budget: u32,
    ) -> crysirror_core::Result<VmYield> {
        let result = self.run(host, budget);
        match result {
            Ok(v) => Ok(v),
            Err(e) => {
                let message = format!("{e:#}");
                self.error = Some(message.clone());
                Err(Error::Io(message))
            }
        }
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported(
            "CatSystem2 framework snapshots are not implemented",
        ))
    }
    fn restore(&mut self, _bytes: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported(
            "CatSystem2 framework snapshots are not implemented",
        ))
    }
    fn diagnostics(&self) -> Vec<u8> {
        serde_json::to_vec(&json!({"profile":"kcs-v2","script":self.config.script,"loaded":self.program.is_some(),"pc":self.machine.as_ref().map(|m|m.pc),"instructions":self.machine.as_ref().map(|m|m.instructions),"pending_native":self.machine.as_ref().and_then(|m|m.pending.as_ref()),"error":self.error})).unwrap_or_default()
    }
}
#[cfg(feature = "runtime")]
pub fn register(registry: &mut crysirror_runtime::AdapterRegistry) -> Result<()> {
    use crysirror_runtime::{model::ClientSpec, runtime::VmFactory};
    struct Factory;
    impl VmFactory for Factory {
        fn create(&self, client: ClientId, spec: &ClientSpec) -> Result<Box<dyn VmAdapter>> {
            ensure!(
                spec.program.is_empty(),
                "CatSystem2 executes original KCS, not data VM actions"
            );
            Ok(Box::new(CatSystem2Vm::new(
                client,
                serde_json::from_value(spec.config.clone())?,
            )?))
        }
    }
    registry.register("catsystem2", std::sync::Arc::new(Factory))
}
