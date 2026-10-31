// Copyright (c) 2026 CrySirror contributors. MIT; see LICENSE/SOURCE.json.
//! Thin native policies over shared capability storage, Canvas and clock.
//! Unsupported message/resource/graphics operations remain pending at their PC.
use crate::rng_policy::NativeRng;
use crate::settings_policy::{MajiroSettingsProfile, NativeSettings, SETTINGS_FILE};
use crate::{
    machine::{Event, Machine, Value},
    program::{Program, MAX_MODULE_BYTES},
};
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{HostServices, InputFrame, StorageWrite, StorageWriteState},
    state::ClientId,
    vm::{VmAdapter, VmYield},
    Error,
};
use crysirror_runtime::{model::ClientSpec, runtime::VmFactory, AdapterRegistry};
use crysirror_tasks::{Task, TaskPool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};
fn mode() -> i32 {
    1
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub script: String,
    /// Original ENV resource selected by the archive bootstrap, never a synthetic script.
    #[serde(default)]
    pub environment: Option<String>,
    /// Explicit portable settings scope, distinct from the original Windows registry.
    #[serde(default)]
    pub settings_profile: MajiroSettingsProfile,
    /// Optional caller-owned module inspected only to guard the reviewed native ABI.
    #[serde(default)]
    pub executable: Option<String>,
    /// Original startup mode. Release play is 1; native is_testplay is mode!=1.
    #[serde(default = "mode")]
    pub run_mode: i32,
    /// Portable logical engine locks, separate from physical keyboard control.
    #[serde(default)]
    pub keyboard_lock_policy: crate::keyboard_policy::KeyboardLockPolicy,
}
pub struct MajiroVm {
    client: ClientId,
    config: Config,
    pool: TaskPool,
    load: Option<Task<Prepared>>,
    machine: Option<Machine>,
    native_abi: bool,
    rng: Option<NativeRng>,
    settings: Option<NativeSettings>,
    settings_write: Option<PendingSettingsWrite>,
    next_page: u32,
    pages: BTreeMap<u32, [u32; 2]>,
    auto_mode_config: i32,
    fast_disable_depth: i32,
    wait: Option<u64>,
    api_calls: u64,
    last_error: Option<String>,
}
struct Prepared {
    program: Program,
    native_abi: bool,
    seed: u32,
    settings: Option<NativeSettings>,
}
struct PendingSettingsWrite {
    pc: u32,
    write: Box<dyn StorageWrite>,
    candidate: NativeSettings,
    output: Vec<Value>,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.keyboard_lock_policy==crate::keyboard_policy::KeyboardLockPolicy::EngineLogical,"physical Windows keyboard-lock control is not supported");

        ensure!(
            VfsPath::new(Mount::Asset, &self.script)?.relative() == self.script
                && self.script.to_ascii_lowercase().ends_with(".mjo"),
            "canonical original Majiro script path"
        );
        if let Some(path) = &self.executable {
            ensure!(
                VfsPath::new(Mount::Asset, path)?.relative() == path,
                "canonical Majiro executable metadata path"
            );
        }
        if let Some(path) = &self.environment {
            ensure!(
                VfsPath::new(Mount::Asset, path)?.relative() == path,
                "canonical Majiro ENV path"
            );
        }
        ensure!(
            self.settings_profile == MajiroSettingsProfile::PortableClient,
            "strict original Windows registry import is unsupported"
        );
        Ok(())
    }
}
impl MajiroVm {
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            client,
            config,
            pool: TaskPool::new("majiro-program", 1, 8)?,
            load: None,
            machine: None,
            native_abi: false,
            rng: None,
            settings: None,
            settings_write: None,
            next_page: 1,
            pages: BTreeMap::new(),
            auto_mode_config: 0,
            fast_disable_depth: 0,
            wait: None,
            api_calls: 0,
            last_error: None,
        })
    }
    fn native(&mut self, h: &mut dyn HostServices, event: &Event) -> Result<Option<Vec<Value>>> {
        let Event::Native {
            hash,
            arguments: a,
            returns,
            pc,
        } = event
        else {
            bail!("Majiro message layout/control mapping is not implemented")
        };
        ensure!(
            self.native_abi,
            "Majiro executable native ABI has not been reviewed"
        );
        if self.settings_write.is_some() {
            return self.poll_settings_write(*pc);
        }
        match *hash {
   // Native executable SHA in SOURCE, sub_410060 branch0x0ebc053b stores
   // the top integer in dword_49b020. This is native policy, not a fake service.
   0x0ebc053b=>{ensure!(!returns&&a.len()==1,"auto_mode_config ABI");self.auto_mode_config=a[0].integer()?;Ok(Some(vec![]))},
   0x6e6c641a=>{ensure!(*returns&&a.is_empty(),"is_testplay ABI");Ok(Some(vec![Value::Integer(i32::from(self.config.run_mode!=1))]))},
   0x81ce0485 => {
    ensure!(a.is_empty(),"fast_mode_disable ABI");
    let depth=self.fast_disable_depth.wrapping_add(1);
    let mut state=h.keyboard_locks(self.client).map_err(core_error)?.logical;
    state.caps_lock=false;
    let committed=h.set_keyboard_locks(self.client,state).map_err(core_error)?;
    ensure!(committed.client==self.client && !committed.logical.caps_lock,"keyboard lock owner completion");
    self.fast_disable_depth=depth;
    self.api_calls+=1;
    Ok(Some(if *returns {vec![Value::Integer(0)]} else {vec![]}))
   },
   0xc6282f19 => {
    ensure!(*returns && a.len()==1, "rand ABI");
    let bound=a[0].integer()?;
    let value=self.rng.as_mut().context("Majiro native RNG seed dependency is not published")?.rand_bound(bound)?;
    Ok(Some(vec![Value::Integer(value)]))
   },
   0x111bd910|0xe5c70196|0xee154520=>{
    ensure!(a.is_empty(),"native speed getter ABI");
    let settings=self.settings.as_ref().context("Majiro speed getter requires original ENV and published portable settings")?;
    let value=match *hash {0x111bd910=>settings.effect_speed,0xe5c70196=>settings.moji_speed,_=>settings.auto_speed};
    Ok(Some(if *returns{vec![Value::Integer(value)]}else{vec![]}))
   },
   0x80177286=>{
    ensure!(a.len()==2,"native reg_read ABI");
    let name=encoding_rs::SHIFT_JIS.decode_without_bom_handling_and_without_replacement(a[0].bytes()?).context("native registry key encoding")?;
    let settings=self.settings.as_ref().context("Majiro reg_read requires published portable settings")?;
    let value=settings.registry_integer(&name,a[1].integer()?)?;
    Ok(Some(if *returns{vec![Value::Integer(value)]}else{vec![]}))
   },
   0x0c93fcb4|0x6501cc30|0x9a1aceca=>{
    let settings=self.settings.as_ref().context("Majiro setting write requires original ENV and portable settings")?;
    let (name,value,current)=if *hash==0x9a1aceca {
     ensure!(a.len()==2,"native reg_write ABI");
     let name=encoding_rs::SHIFT_JIS.decode_without_bom_handling_and_without_replacement(a[0].bytes()?).context("native registry key encoding")?.into_owned();
     (name,a[1].integer()?,false)
    }else{ensure!(a.len()==1,"native speed setter ABI");(if *hash==0x0c93fcb4{"EffectSpeed"}else{"MojiSpeed"}.to_owned(),a[0].integer()?,true)};
    let candidate=settings.candidate_integer(&name,value,current)?;
    let storage=h.storage(self.client).map_err(core_error)?;
    let path=VfsPath::new(Mount::Save,SETTINGS_FILE)?;
    let write=storage.begin_write_atomic(&path,Arc::from(candidate.bytes()?)).map_err(core_error)?;
    self.settings_write=Some(PendingSettingsWrite{pc:*pc,write,candidate,output:if *returns{vec![Value::Integer(0)]}else{vec![]}});
    self.poll_settings_write(*pc)
   },
   // Original sub_419500/sub_4404a0: 24bpp BGR allocation, zero-filled storage.
   // Indexed8bpp is a distinct palette resource and must not be approximated.
   0x83d81f59=>{
    ensure!(*returns&&(a.len()==2||a.len()==3),"page_create ABI");
    ensure!(a.len()==2||a[2].integer()?!=0,"Majiro indexed page needs shared palette resource");
    let w=a[0].integer()?;let height=a[1].integer()?;
    ensure!(w>0&&height>0&&w<=8192&&height<=8192,"Majiro page geometry profile");
    ensure!(self.pages.len()<256,"Majiro page budget");let id=self.next_page;
    let batch=json!({"canvases":[{"op":"create","id":id,"width":w,"height":height},{"op":"clear","id":id,"color":[0,0,0,255]}]});
    h.submit_engine(self.client,1,&serde_json::to_vec(&batch)?).map_err(core_error)?;
    self.pages.insert(id,[w as u32,height as u32]);self.next_page=id.checked_add(1).context("Majiro page id overflow")?;self.api_calls+=1;Ok(Some(vec![Value::Integer(id as i32)]))
   },
   _=>bail!("unsupported original Majiro syscall {hash:#010x}; pending call was not acknowledged"),
  }
    }
    fn poll_settings_write(&mut self, pc: u32) -> Result<Option<Vec<Value>>> {
        let pending = self
            .settings_write
            .as_mut()
            .context("missing native settings receipt")?;
        ensure!(pending.pc == pc, "stale native settings receipt");
        match pending.write.poll().map_err(core_error)? {
            StorageWriteState::Pending => Ok(None),
            StorageWriteState::Failed(error) => {
                bail!("Majiro shared settings write failed: {error}")
            }
            StorageWriteState::Ready => {
                let pending = self.settings_write.take().unwrap();
                self.settings = Some(pending.candidate);
                Ok(Some(pending.output))
            }
        }
    }
    fn run(&mut self, h: &mut dyn HostServices, budget: u32) -> Result<VmYield> {
        if self.machine.is_none() {
            if self.load.is_none() {
                let storage = h.storage(self.client).map_err(core_error)?;
                let path = VfsPath::new(Mount::Asset, &self.config.script)?;
                let executable = self.config.executable.clone();
                let environment = self.config.environment.clone();
                self.load = Some(self.pool.spawn(move |cancel| {
                    cancel.check()?;
                    // Original startup424d06 calls srand(time(0)). Capture it
                    // once on this startup dependency, never from elapsed_ms
                    // or on each native rand call.
                    let seed = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .context("Majiro native Unix-second seed unavailable")?
                        .as_secs() as u32;
                    let reader = storage.open_asset(&path)?;
                    let len = reader.len()?;
                    ensure!(
                        len >= 32 && len <= MAX_MODULE_BYTES as u64,
                        "Majiro MJO source budget"
                    );
                    let mut bytes = vec![0; len as usize];
                    let mut at = 0;
                    while at < bytes.len() {
                        cancel.check()?;
                        let end = (at + 65536).min(bytes.len());
                        let n = reader.read_at(at as u64, &mut bytes[at..end])?;
                        ensure!(n > 0 && n <= end - at, "Majiro short source read");
                        at += n;
                    }
                    cancel.check()?;
                    let p = Program::parse(&bytes)?;
                    cancel.check()?;
                    let native_abi = if let Some(executable) = executable {
                        let path = VfsPath::new(Mount::Asset, &executable)?;
                        let reader = storage.open_asset(&path)?;
                        let bytes =
                            crysirror_resource::read_all(reader.as_ref(), 16 * 1024 * 1024)?;
                        cancel.check()?;
                        let pe = crysirror_resource::pe::PeData::parse(&bytes)?;
                        pe.sha256()
                            == [
                                0x75, 0xf2, 0x89, 0xd8, 0xd5, 0xb7, 0x30, 0xc6, 0xf3, 0x32, 0x48,
                                0x8d, 0xf8, 0xfe, 0x48, 0x27, 0xd7, 0xe9, 0x5d, 0x1a, 0xf4, 0xa1,
                                0x00, 0xbf, 0xd2, 0xa8, 0x80, 0xa0, 0xb9, 0xe8, 0x06, 0x02,
                            ]
                    } else {
                        true
                    };
                    let settings = if let Some(environment) = environment {
                        let path = VfsPath::new(Mount::Asset, &environment)?;
                        let reader = storage.open_asset(&path)?;
                        let bytes = crysirror_resource::read_all(reader.as_ref(), 256 * 1024)?;
                        cancel.check()?;
                        Some(NativeSettings::load(&bytes, storage.as_ref())?)
                    } else {
                        None
                    };
                    cancel.check()?;
                    Ok(Prepared {
                        program: p,
                        native_abi,
                        seed,
                        settings,
                    })
                })?);
            }
            let Some(prepared) = self.load.as_mut().unwrap().try_take()? else {
                return Ok(VmYield::WaitUntil(h.now_ms().saturating_add(1)));
            };
            self.machine = Some(Machine::new(Arc::new(prepared.program)));
            self.native_abi = prepared.native_abi;
            self.rng = Some(NativeRng::new(prepared.seed));
            self.settings = prepared.settings;
            self.load = None;
        }
        if let Some(t) = self.wait {
            if h.now_ms() < t {
                return Ok(VmYield::WaitUntil(t));
            }
            self.wait = None;
        }
        let mut m = self.machine.take().unwrap();
        let result = (|| {
            for _ in 0..budget {
                if let Some(event) = m.step()? {
                    if event == Event::Halted {
                        return Ok(VmYield::Halted);
                    }
                    let output = self.native(h, &event).with_context(|| {
                        format!(
                            "Majiro {} pc={:#x} line={}",
                            self.config.script, m.pc, m.line
                        )
                    })?;
                    let Some(output) = output else {
                        return Ok(VmYield::WaitUntil(h.now_ms().saturating_add(1)));
                    };
                    m.acknowledge(output)?;
                }
            }
            Ok(VmYield::BudgetExhausted)
        })();
        self.machine = Some(m);
        result
    }
}
fn core_error(e: Error) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}
impl VmAdapter for MajiroVm {
    fn engine_id(&self) -> &'static str {
        "majiro"
    }
    fn snapshot_schema(&self) -> u32 {
        1
    }
    fn step(
        &mut self,
        h: &mut dyn HostServices,
        _: InputFrame,
        budget: u32,
    ) -> crysirror_core::Result<VmYield> {
        if self.last_error.is_some() {
            return Err(Error::Unsupported(
                "Majiro stopped after failed native operation",
            ));
        }
        self.run(h, budget).map_err(|e| {
            self.last_error = Some(format!("{e:#}"));
            Error::Io(format!("{e:#}"))
        })
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported(
            "Majiro resource/VM checkpoint reconstruction",
        ))
    }
    fn restore(&mut self, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported(
            "Majiro resource/VM checkpoint reconstruction",
        ))
    }
    fn diagnostics(&self) -> Vec<u8> {
        serde_json::to_vec(&json!({"vm":"original-majiro-mjo","pc":self.machine.as_ref().map(|m|m.pc),"line":self.machine.as_ref().map(|m|m.line),"instructions":self.machine.as_ref().map(|m|m.executed),"engine_api_calls":self.api_calls,"pages":self.pages.len(),"native_profile_enabled":self.native_abi,"rng":self.rng.as_ref().map(NativeRng::snapshot),"settings":{"profile":self.config.settings_profile,"import_performed":false,"loaded":self.settings.is_some(),"source":self.settings.as_ref().map(|s|if s.loaded_portable_file{"portable_client_save"}else{"native_env_defaults"}),"entries":self.settings.as_ref().map(NativeSettings::entries),"pending_write":self.settings_write.is_some(),"commits":self.settings.as_ref().map(|s|s.commits)},"module_guard":if self.config.executable.is_some(){"sha256"}else{"not_requested"},"auto_mode_config":self.auto_mode_config,"fast_disable_depth":self.fast_disable_depth,"keyboard_locks":{"policy":self.config.keyboard_lock_policy,"physical_imported":false,"physical_control":false},"run_mode":self.config.run_mode,"error":self.last_error,"owns_framebuffer":false})).unwrap_or_default()
    }
}
struct Factory;
impl VmFactory for Factory {
    fn create(&self, client: ClientId, spec: &ClientSpec) -> Result<Box<dyn VmAdapter>> {
        ensure!(
            spec.program.is_empty(),
            "Majiro executes original MJO, not data-VM actions"
        );
        Ok(Box::new(MajiroVm::new(
            client,
            serde_json::from_value(spec.config.clone())?,
        )?))
    }
}
pub fn register(registry: &mut AdapterRegistry) -> Result<()> {
    registry.register("majiro", Arc::new(Factory))
}

#[cfg(test)]
mod settings_tests;
