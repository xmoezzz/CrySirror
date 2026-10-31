// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Dormant installed-game factory. Only original VM policy is prepared here;
//! image decoding, composition, font resources, audio and clocks are services.
use crate::{
    adapter::{NativeAdapter, ScriptLoader},
    bridge::{Bridge, CallbackProfile, SceneBindings},
    native_audio::AudioBindings,
    native_bootstrap::{BootEntry, PreparedBoot},
    native_resources::ResourceNames,
    native_services_bootstrap::{
        resolved_window_resource, NativeWindowSource, PreparedNativeServices,
    },
    native_text::{MessageBindings, MessageWindow, NativeTextBridge, TextFace},
    native_ui::SelectionProfile,
    resource_bridge::{NativeResources, SourceOpener},
    Program,
};
use anyhow::{ensure, Context, Result};
use crysirror_core::{
    image_processing::{ImageInput, ImageJob, ImageProcessState},
    path::{Mount, VfsPath},
    service::{CivilTimeQuery, CivilTimeState, ClientStorage, HostServices, InputFrame},
    state::ClientId,
    vm::{VmAdapter, VmYield},
    Error,
};
use crysirror_runtime::{model::ClientSpec, runtime::VmFactory, AdapterRegistry};
use crysirror_tasks::{Cancellation, Task, TaskPool};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
const SIZE: [u32; 2] = [800, 600];
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Entry {
    #[default]
    Controller,
    NewGame,
}
/// Paths refer to the client capability namespace, including mounted installer
/// directories. The reviewed callback ABI is authenticated by PreparedBoot.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub root: String,
    pub module: String,
    #[serde(default = "default_font")]
    pub font: String,
    #[serde(default)]
    pub entry: Entry,
}
fn default_font() -> String {
    "default".into()
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        let names = ResourceNames::new(&self.root)?;
        names.path(&self.module)?;
        ensure!(
            !self.font.is_empty() && self.font.len() <= 256,
            "System-NNN font alias"
        );
        Ok(())
    }
}
struct Factory;
impl VmFactory for Factory {
    fn create(&self, client: ClientId, spec: &ClientSpec) -> Result<Box<dyn VmAdapter>> {
        ensure!(
            spec.program.is_empty(),
            "System-NNN executes original SPT; no data VM program"
        );
        Ok(Box::new(InstalledAdapter::new(
            client,
            serde_json::from_value(spec.config.clone())?,
        )?))
    }
}
pub fn register(registry: &mut AdapterRegistry) -> Result<()> {
    registry.register("system-nnn", Arc::new(Factory))
}
struct WindowInput {
    source: NativeWindowSource,
    input: Option<ImageInput>,
    effects: Arc<[u8]>,
    job: Option<ImageJob>,
    resolved: Option<MessageWindow>,
}
struct Prepared {
    boot: PreparedBoot,
    loader: ScriptLoader,
    services: PreparedNativeServices,
    resources: NativeResources,
    windows: Vec<WindowInput>,
    startup:Option<crate::native_shell::NativeStartupProfile>,
    back_program:Option<Arc<Program>>,
}
/// No source read, task thread, engine mutation or decoder admission at creation.
pub struct InstalledAdapter {
    client: ClientId,
    config: Config,
    pool: Option<TaskPool>,
    task: Option<Task<Prepared>>,
    civil: Option<Box<dyn CivilTimeQuery>>,
    clock: Option<u32>,
    prepared: Option<Prepared>,
    vm: Option<NativeAdapter>,
    failure: Option<String>,
    published: Vec<String>,
    started: bool,
    shell:Option<crate::native_shell::NativeStartupShell>,
}
fn read(
    storage: &dyn ClientStorage,
    path: &str,
    limit: u64,
    cancel: &Cancellation,
) -> Result<Option<Vec<u8>>> {
    cancel.check()?;
    let path = VfsPath::new(Mount::Asset, path)?;
    let source = match storage.open_asset(&path) {
        Ok(source) => source,
        Err(Error::NotFound) => return Ok(None),
        Err(reason) => return Err(reason.into()),
    };
    let bytes = crysirror_resource::read_all(source.as_ref(), limit)?;
    cancel.check()?;
    Ok(Some(bytes))
}
impl InstalledAdapter {
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        ensure!(client.0 != 0, "System-NNN installed client identity");
        Ok(Self {
            client,
            config,
            pool: None,
            task: None,
            civil: None,
            clock: None,
            prepared: None,
            vm: None,
            failure: None,
            published: vec![],
            started: false,
            shell:None,
        })
    }
    /// Only work admitted by a prior ordinary step is a dependency. This does
    /// not turn native timer or input waits into offline busy work.
    pub fn dependency_pending(&self)->bool {
        if self.failure.is_some()||!self.started{return false}
        self.task.is_some()||self.civil.is_some()||self.prepared.is_some()
            ||self.shell.as_ref().is_some_and(|s|s.dependency_pending())
            ||self.vm.as_ref().is_some_and(|vm|vm.dependency_pending())
    }
    /// Independent owner completion pump. No fresh VM batch, clock advancement
    /// or input sampling occurs here; the caller remains free to yield.
    pub fn poll_dependency(&mut self,host:&mut dyn HostServices)->crysirror_core::Result<()> {
        if let Some(reason)=&self.failure{return Err(Error::Io(reason.clone()))}
        if !self.dependency_pending(){return Ok(())}
        let result:Result<()>=if self.vm.is_none(){self.advance(host,InputFrame::default(),1).map(|_|())}
        else if self.shell.as_ref().is_some_and(|s|s.dependency_pending()){
            self.shell.as_mut().unwrap().poll(host,InputFrame::default()).map(|_|())
        }else{self.vm.as_mut().unwrap().poll_dependency(host).map_err(Into::into)};
        if let Err(reason)=result {
            let reason=format!("System-NNN installed bootstrap/VM: {reason:#}");
            if self.vm.is_none(){for path in self.published.drain(..){let _=host.release_image_resource(self.client,&path);}}
            self.failure=Some(reason.clone());return Err(Error::Io(reason));
        }
        Ok(())
    }
    fn start(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let pool = TaskPool::new("nnn-installed-bootstrap", 1, 1)?;
        let permit = pool.reserve()?;
        let storage = host.storage(self.client)?;
        self.civil = Some(host.begin_civil_time(self.client)?);
        let config = self.config.clone();
        let client = self.client;
        self.task = Some(permit.spawn(move |cancel| {
            let boot = PreparedBoot::mugen2(
                |path, limit| read(storage.as_ref(), path, limit, &cancel),
                &config.root,
                &config.module,
                match config.entry {
                    Entry::Controller => BootEntry::Controller,
                    Entry::NewGame => BootEntry::NewGame,
                },
                &cancel,
            )?;
            let module = read(
                storage.as_ref(),
                &boot.resources.path(&config.module)?,
                32 << 20,
                &cancel,
            )?
            .context("NNN callback module missing after boot")?;
            let services = PreparedNativeServices::from_configuration(
                &boot.configuration(),
                &module,
                storage.as_ref(),
                AudioBindings {
                    music: 0,
                    sound_base: 16,
                    voice_base: 32,
                },
                false,
                &cancel,
            )?;
            let io = storage.clone();
            let opener: SourceOpener = Arc::new(move |path, cancel| {
                cancel.check()?;
                match io.open_asset(&VfsPath::new(Mount::Asset, path)?) {
                    Ok(source) => Ok(Some(source)),
                    Err(Error::NotFound) => Ok(None),
                    Err(reason) => Err(reason.into()),
                }
            });
            let resources = NativeResources::blackcyc(client, opener)?;
            let startup=if matches!(config.entry,Entry::Controller) {Some(crate::native_shell::NativeStartupProfile::read(&boot.configuration(),storage.as_ref(),&cancel)?)}else{None};
            let back_program=if let Some(startup)=&startup {Some(boot.load_script(&startup.opening_script,|path,limit|read(storage.as_ref(),path,limit,&cancel),&cancel)?)}else{None};
            let levels = u32::try_from(boot.setup.integer(b"dwqPackLevel")?.unwrap_or(0))?;
            let mut windows = vec![];
            if let Some(native) = services.windows(0, levels)? {
                for source in std::iter::once(native.body).chain(native.name) {
                    let decoded = resources.image_source(&source.picture, &cancel)?;
                    let effects = if decoded.has_mask {
                        Vec::new()
                    } else {
                        vec![crysirror_image::effects::ImageEffect::SpriteAlpha {
                            spec: crysirror_image::sprite_alpha::SpriteAlpha::CornerKey {
                                right: false,
                            },
                        }]
                    };
                    windows.push(WindowInput {
                        source,
                        input: Some(decoded.input),
                        effects: serde_json::to_vec(&effects)?.into(),
                        job: None,
                        resolved: None,
                    });
                }
            }
            let names = boot.resources.clone();
            let project = boot.project.clone();
            let scripts = boot.scripts;
            let loader: ScriptLoader = Arc::new(move |name, cancel| {
                ensure!(
                    project.index(name).is_some(),
                    "NNN script absent from original project"
                );
                let path = names.script(name)?;
                let bytes = read(storage.as_ref(), &path, 16 << 20, cancel)?
                    .with_context(|| format!("NNN script absent: {path}"))?;
                Ok(Arc::new(Program::parse(&bytes, scripts)?))
            });
            cancel.check()?;
            Ok(Prepared {
                boot,
                loader,
                services,
                resources,
                windows,startup,back_program,
            })
        })?);
        self.pool = Some(pool);
        self.started = true;
        Ok(())
    }
    fn advance(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
        budget: u32,
    ) -> Result<VmYield> {
        if let Some(vm) = &mut self.vm {
            if let Some(shell)=&mut self.shell {
                if !shell.poll(host,input)? {return Ok(VmYield::BudgetExhausted)}
                let yielded=vm.step(host,input,budget)?;
                ensure!(yielded!=VmYield::Halted,"NNN original Opening background SPT completed; native printmovie/Title continuation requires the outer transition implementation");
                return Ok(yielded);
            }
            return Ok(vm.step(host, input, budget)?);
        }
        if budget == 0 {
            return Ok(VmYield::BudgetExhausted);
        }
        if !self.started {
            self.start(host)?;
            return Ok(VmYield::BudgetExhausted);
        }
        if let Some(clock) = &mut self.civil {
            match clock.poll()? {
                CivilTimeState::Pending => (),
                CivilTimeState::Failed(reason) => anyhow::bail!("NNN host clock: {reason}"),
                CivilTimeState::Ready(time) => {
                    ensure!(time.client == self.client, "NNN clock scope");
                    self.clock = Some(u32::try_from(time.unix_ms.div_euclid(1000))?);
                    self.civil = None;
                }
            }
        }
        if let Some(task) = &mut self.task {
            if let Some(prepared) = task.try_take()? {
                self.prepared = Some(prepared);
                self.task = None;
            }
        }
        let Some(prepared) = &mut self.prepared else {
            return Ok(VmYield::BudgetExhausted);
        };
        if self.clock.is_none() {
            return Ok(VmYield::BudgetExhausted);
        }
        let images = host.image_metadata(self.client)?;
        let mut pending = false;
        for (index, window) in prepared.windows.iter_mut().enumerate() {
            if window.resolved.is_some() {
                continue;
            }
            if let Some(source) = window.input.take() {
                window.job = Some(images.begin_process(source, window.effects.clone())?);
            }
            match window
                .job
                .as_mut()
                .context("NNN window image job missing")?
                .poll()?
            {
                ImageProcessState::Pending => pending = true,
                ImageProcessState::Failed(reason) => {
                    anyhow::bail!("NNN native message window: {reason}")
                }
                ImageProcessState::Ready(image) => {
                    ensure!(image.client() == self.client, "NNN window image scope");
                    let published = host.publish_image_resource(
                        self.client,
                        0x66000000 + index as u32,
                        image.clone(),
                    )?;
                    let path = published.path.relative().to_owned();
                    self.published.push(path.clone());
                    window.resolved = Some(resolved_window_resource(
                        5002 + index as u32,
                        &window.source,
                        path,
                        image,
                    )?);
                    window.job = None;
                }
            }
        }
        if pending {
            return Ok(VmYield::BudgetExhausted);
        }
        // All fallible resource work has completed before publishing the scene.
        let mut prepared = self.prepared.take().unwrap();
        prepared.boot.initialize_random_state(self.clock, None)?;
        let body = prepared
            .windows
            .first()
            .and_then(|window| window.resolved.clone());
        let name = prepared
            .windows
            .get(1)
            .and_then(|window| window.resolved.clone());
        let profile = prepared.services.message_profile_with_windows(
            MessageBindings {
                client: self.client,
                root: 1,
                body_layer: 5000,
                name_layer: 5001,
                order: 1000,
                canvas: SIZE,
                window: body,
            },
            &self.config.font,
            1,
            name,
        )?;
        host.text_measurement(self.client)?.measure_simple(
            &self.config.font,
            profile.body.size as f32,
            "",
        )?;
        let selection = prepared.services.selection_bridge(SelectionProfile {
            client: self.client,
            root: 1,
            widget_base: 6000,
            layer_base: 6000,
            order: 1100,
            canvas: SIZE,
            face: TextFace {
                font: self.config.font.clone(),
                size: profile.body.size,
                spacing: profile.body.spacing,
                color: profile.body.color,
            },
            rows: vec![],
            message_origins: vec![],
            single_byte_map: None,
        })?;
        let mut bridge = Bridge::new(
            self.client,
            CallbackProfile::Mugen2Legacy,
            SceneBindings {
                root: 1,
                overlap_canvas: 7,
                effect_base: 100,
                size: SIZE,
                layers: 16,
                music_channel: 0,
            },
        )?;
        let frame_time = prepared.boot.setup.integer(b"frameTime")?.unwrap_or(50);
        bridge.set_frame_time(u64::try_from(frame_time)?)?;
        bridge.configure_text(NativeTextBridge::new(profile)?)?;
        bridge.configure_selection(selection)?;
        bridge.configure_resources(
            prepared.resources.clone(),
            prepared.boot.resources.clone(),
            u32::try_from(prepared.boot.setup.integer(b"dwqPackLevel")?.unwrap_or(0))?,
        )?;
        let resources = prepared.resources.clone();
        let names = prepared.services.resources.clone();
        bridge.configure_audio(crate::audio_bridge::AudioBridge::new(
            self.client,
            prepared.services.audio,
            prepared.services.music_names,
            prepared.services.sound_names,
            Arc::new(move |source, cancel| resources.audio(source, &names, cancel)),
        )?)?;
        let vm=if let Some(startup)=prepared.startup.take() {
            let frame_ms=u64::try_from(frame_time)?;
            let shell=crate::native_shell::NativeStartupShell::new(self.client,1,SIZE,frame_ms,u32::try_from(prepared.services.preferences.launch_count)?,startup.clone(),prepared.resources)?;
            let vm=NativeAdapter::from_background(prepared.boot,prepared.loader,bridge,startup.opening_script.clone(),prepared.back_program.take().context("NNN opening background SPT not prepared")?,crate::native_shell::NativeMode::Opening,startup.opening_loop)?;
            self.shell=Some(shell);vm
        }else{NativeAdapter::from_bootstrap(prepared.boot,prepared.loader,bridge)?};
        host.submit_engine(self.client,1,&serde_json::to_vec(&serde_json::json!({"layers":[{"op":"put","layer":{"id":1,"rect":[0,0,800,600],"paint":{"kind":"solid","color":[0,0,0,255]}}}]}))?)?;
        self.vm = Some(vm);
        Ok(VmYield::BudgetExhausted)
    }
}
impl VmAdapter for InstalledAdapter {
    fn preparation_pending(&self)->bool {self.failure.is_none()&&(self.vm.is_none()||self.shell.as_ref().is_some_and(|s|s.preparation_pending()))}
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
        if let Some(reason) = &self.failure {
            return Err(Error::Io(reason.clone()));
        }
        match self.advance(host, input, budget) {
            Ok(result) => Ok(result),
            Err(reason) => {
                let reason = format!("System-NNN installed bootstrap/VM: {reason:#}");
                if self.vm.is_none() {
                    for path in self.published.drain(..) {
                        let _ = host.release_image_resource(self.client, &path);
                    }
                }
                self.failure = Some(reason.clone());
                Err(Error::Io(reason))
            }
        }
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported(
            "System-NNN original resource/save reconstruction",
        ))
    }
    fn restore(&mut self, _bytes: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported(
            "System-NNN original resource/save reconstruction",
        ))
    }
    fn diagnostics(&self) -> Vec<u8> {
        let mut value=self.vm.as_ref().and_then(|vm|serde_json::from_slice::<serde_json::Value>(&vm.diagnostics()).ok()).unwrap_or_else(||serde_json::json!({"phase":if self.started{"preparing"}else{"dormant"},"owns_framebuffer":false}));
        if let Some(shell)=&self.shell {value["native_shell"]=shell.diagnostics();value["dependency_pending"]=serde_json::json!(shell.dependency_pending()||self.vm.as_ref().is_some_and(|vm|vm.dependency_pending()));}
        value["factory_registered"] = true.into();
        value["bootstrap_pending"] = (self.task.is_some() || self.prepared.is_some()).into();
        value["error"] = serde_json::json!(self.failure);
        serde_json::to_vec(&value).unwrap_or_default()
    }
}
