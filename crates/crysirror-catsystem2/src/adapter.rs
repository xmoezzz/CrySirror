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
use crysirror_resource::xml::XmlDocument;
use crysirror_tasks::{Task, TaskPool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};
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
    input_routes_bound:bool,
    event_wait:Option<NativeEventWait>,
    image_pool_loading:Option<crate::image_list::PendingImages>,
    debug_key_absent: bool,
    menu_installed: bool,
    tasks: crate::task_policy::NativeTaskPolicy,
    xml_loading: Option<Task<Option<Arc<XmlDocument>>>>,
    loop_loading: Option<Task<PreparedLoops>>,
    settings_loading: Option<Task<crate::native_settings::NativeSettings>>,
    settings: Option<crate::native_settings::NativeSettings>,
    loop_catalog_revision: u64,
    loop_entries: u32,
    documents: BTreeMap<u32, Arc<XmlDocument>>,
    next_document: u32,
    document_bytes: usize,
    native_calls: BTreeMap<u32, u64>,
    interpreter: Option<crate::native_interpreter::Interpreter>,
    split_loading: Option<Task<crysirror_text::byte_split::ByteSplitOutput>>,
    save_loading: Option<Task<Option<crate::native_save::Container>>>,
    saves: BTreeMap<u32,crate::native_save::Container>,
    next_save:u32,
    string_variables:crate::native_values::StringVariables,
    sound_loading: Option<Task<crate::sound_definition::SoundDefinition>>,
    sound_definition: Option<crate::sound_definition::SoundDefinition>,
}
#[derive(Clone,Debug)]
struct NativeEventWait{slot:u32,pc:u32}
struct PreparedLoops { entries: Option<Vec<crysirror_core::service::AudioLoopMetadata>>, source: String }
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
            input_routes_bound:false,
            event_wait:None,
            image_pool_loading:None,
            debug_key_absent: false,
            menu_installed: false,
            tasks: crate::task_policy::NativeTaskPolicy::bootstrap()?,
            xml_loading: None,
            loop_loading: None,
            settings_loading: None,
            settings: None,
            loop_catalog_revision: 0,
            loop_entries: 0,
            documents: BTreeMap::new(),
            next_document: 1,
            document_bytes: 0,
            native_calls: BTreeMap::new(),
            interpreter: None,
            split_loading: None,
            save_loading:None,
            saves:BTreeMap::new(),
            next_save:1,
            string_variables:Default::default(),
            sound_loading: None,
            sound_definition: None,
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
            // Runtime's startup fence may poll only dormant adapters. Source
            // publication must not also execute the first KCS instructions;
            // they belong to the first ordinary owner tick and its input.
            return Ok(VmYield::BudgetExhausted);
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
        if self.input_routes_bound {
            let mut drained=false;
            for _ in 0..16 {
            let delivery:Option<crysirror_scene::input_routes::InputDelivery>=serde_json::from_slice(&host.query_engine(self.client,EngineQuery::InputRouteDelivery)?)?;
            if let Some(delivery)=delivery {
                if self.event_wait.is_none()&&matches!(delivery.event.phase,crysirror_scene::input_routes::InputRoutePhase::HoverEnter|crysirror_scene::input_routes::InputRoutePhase::HoverLeave) {
                    //5DE8E0 7/8→Main64→877310: no VM callback while
                    //native event-wait flag16 is zero. Only71/65A180 calls
                    //877390 to arm it;the concrete gate is checked above.
                    //An unarmed hover does not execute a script callback.
                    // This concrete native gate is unarmed, including after a
                    // no-event owner tick. No script callback can be resumed.
                    ensure!(self.tasks.button(delivery.receiver)?.binding_target==Some(self.tasks.current()),"CatSystem2 unsupported hover callback target");
                    host.submit_engine(self.client,1,&serde_json::to_vec(&json!({"input_routes":[{"op":"acknowledge","receipt":delivery.receipt,"handled":true}]}))?)?;
                }else{
                    anyhow::bail!("CatSystem2 unsupported native input callback {:?} for task {} at {} original KCS PC {:#x} (event wait {:?}); delivery {:?} remains unacknowledged",delivery.event.phase,delivery.receiver,self.config.script,self.event_wait.as_ref().map_or(self.machine.as_ref().context("CatSystem2 machine missing")?.pc,|w|w.pc),self.event_wait.as_ref().map(|w|w.slot),delivery.receipt);
                }
            }else{drained=true;break;}
            }
            if !drained{return Ok(VmYield::BudgetExhausted);}
            let stats:crysirror_scene::input_routes::InputRouteStats=serde_json::from_slice(&host.query_engine(self.client,EngineQuery::InputRouteStats)?)?;
            if stats.queued_samples>0||stats.queued_events>0{return Ok(VmYield::BudgetExhausted);}
        }
        if self.event_wait.is_some(){
            let mut proposed=self.tasks.clone();
            let receivers=proposed.initialize_fes_tasks(self.config.canvas)?;
            if !receivers.is_empty(){
                let commands=receivers.into_iter().map(|receiver|json!({"op":"put","receiver":receiver})).collect::<Vec<_>>();
                host.submit_engine(self.client,1,&serde_json::to_vec(&json!({"input_routes":commands}))?)?;
                self.input_routes_bound=true;
            }
            self.tasks=proposed;
            //5E84E0→877450→8762A0: a yielded native71 is polled on the
            // next owner task tick even without input, returning stack0.
            self.machine.as_mut().context("CatSystem2 machine missing")?.resume_native_event(None)?;
            self.event_wait=None;
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
                        1 => {
                            ensure!(call.arguments.len()==1,"CatSystem2 debug log arguments");
                            machine.preflight_native(false,&[])?;
                            // 650400 formats VM substitutions, then escapes its
                            // percent bytes for 5CAC00's500-byte printf sink.
                            // The engine log accepts literal text, never printf.
                            let mut bytes=crate::native_values::format_bytes(machine,call.arguments[0])?;
                            ensure!(!bytes.windows(2).any(|w|w==b"%%"),"CatSystem2 unsupported consecutive-percent debug format");
                            ensure!(bytes.len()+bytes.iter().filter(|b|**b==b'%').count()<2047,"CatSystem2 debug format buffer extent");
                            bytes.truncate(500);
                            let text=crysirror_resource::text_stream::decode_auto(&bytes,500)?;
                            host.append_log(self.client,&text)?;
                            machine.acknowledge(None)?;
                        }
                        19..=22 => {
                            ensure!(call.arguments.len()==2,"CatSystem2 string compare arguments");
                            let formatted=matches!(call.id,20|22);
                            let a=if formatted {crate::native_values::format_bytes(machine,call.arguments[0])?} else {machine.read_cstring(call.arguments[1],4096)?};
                            let b=if formatted {crate::native_values::format_bytes(machine,call.arguments[1])?} else {machine.read_cstring(call.arguments[0],4096)?};
                            machine.acknowledge(Some(crate::native_values::compare(&a,&b,call.id>=21)))?;
                        }
                        36 => {
                            // 6556C0: native memset(dst, low-byte value, byte
                            // count), void return; this is owned VM memory.
                            ensure!(call.arguments.len()==3,"CatSystem2 memory fill arguments");
                            let count=call.arguments[2] as usize;
                            ensure!(count<=crate::kcs::MAX_SCRIPT_BYTES,"CatSystem2 native memory fill budget");
                            if count==0 {machine.acknowledge(None)?;}else{
                                machine.commit_native(None,&[(call.arguments[0],vec![call.arguments[1] as u8;count])])?;
                            }
                        }
                        41 => {
                            ensure!(call.arguments.len()==3,"CatSystem2 byte token split arguments");
                            machine.preflight_native(false,&[])?;
                            if self.split_loading.is_none() {
                                let permit=match self.pool.as_ref().context("CatSystem2 worker pool missing")?.reserve() {Ok(p)=>p,Err(e) if e.is::<crysirror_tasks::CapacityExceeded>()=>return Ok(VmYield::BudgetExhausted),Err(e)=>return Err(e)};
                                let source=machine.read_cstring(call.arguments[1],65534)?;
                                let delimiters=machine.read_cstring(call.arguments[2],256)?;
                                self.split_loading=Some(permit.spawn(move |cancel|crysirror_text::byte_split::split_delimited_bytes(&source,&delimiters,crysirror_text::byte_split::ByteSplitBoundary::ShiftJis,Default::default(),&||cancel.check()))?);
                                return Ok(VmYield::BudgetExhausted);
                            }
                            let Some(output)=self.split_loading.as_mut().unwrap().try_take()? else{return Ok(VmYield::BudgetExhausted);};
                            // Native41 returns void. Only the native buffer's
                            // logical address/layout belongs to the adapter.
                            machine.commit_native(None,&[(call.arguments[0],output.nul_bytes)])?;
                            self.split_loading=None;
                        }
                        42 => {
                            ensure!(call.arguments.len()==3,"CatSystem2 NUL-list item arguments");
                            let mut bytes=crate::native_values::nul_list_item(machine,call.arguments[1],call.arguments[2])?;
                            let present=!bytes.is_empty();bytes.extend([0,0]);
                            machine.commit_native(Some(u32::from(present)),&[(call.arguments[0],bytes)])?;
                        }
                        66 => {
                            ensure!(call.arguments.len()==3,"CatSystem2 task constructor arguments");
                            ensure!(matches!(call.arguments[0],2|6|15|34),"CatSystem2 unsupported native gcall 66 task class {} at PC {:#x}; original call remains unacknowledged",call.arguments[0],call.pc);
                            machine.preflight_native(true,&[])?;
                            let name=crate::native_values::format_bytes(machine,call.arguments[2])?;
                            let mut proposed=self.tasks.clone();
                            let id=if call.arguments[0]==2 {
                                let plane=proposed.create_plane(call.arguments[1],name)?;
                                host.submit_engine(self.client,1,&serde_json::to_vec(&json!({"layers":[{"op":"put","layer":plane.layer_spec()?}]}))?)?;
                                plane.id
                            }else if call.arguments[0]==6{
                                //5DD580 class6 is5DE590 alone: unbound button,
                                //not Plane. No geometry/input record exists
                                //until separately reviewed message89 binding.
                                proposed.create_button(call.arguments[1],name)?.id
                            }else if call.arguments[0]==15{
                                //5F3790/68E6F0 create an empty bitmap wrapper
                                //and native task fields, with manager/baseplane
                                //absent. Full5F3AE0/5DC9D0 ignore26. No file IO
                                //or image resource exists before message279.
                                proposed.create_image_list(call.arguments[1],name)?.id
                            }else{
                                //627E50→62DAA0→26. The constructor125 query
                                //has no parent/children to reach INPUT through;
                                //saved cursor remains Unknown, never zeroed.
                                proposed.create_fes(call.arguments[1],name)?
                            };
                            // Original1/2 attach the native task.26 is unhandled
                            // for Plane/Button/ImageList; FES copies the current
                            // main's source-empty parameter buffer after attach.
                            self.tasks=proposed;machine.acknowledge(Some(id))?;
                        }
                        79 => {
                            ensure!(call.arguments.len()==4,"CatSystem2 solid plane arguments");
                            machine.preflight_native(false,&[])?;
                            let id=call.arguments[0];
                            if id!=0 {
                                let w=f32::from_bits(call.arguments[1]);let h=f32::from_bits(call.arguments[2]);
                                ensure!(w.is_finite()&&h.is_finite()&&w>0.&&h>0.&&w<=8192.&&h<=8192.,"CatSystem2 unsupported native solid plane extent");
                                let plane=self.tasks.plane(id)?;
                                ensure!(plane.image.is_none(),"CatSystem2 unsupported native solid replacement of an image plane");
                                let c=call.arguments[3];
                                // 63FDE0→message42→679140 builds two triangles
                                // with XYZ plus packed diffuse ARGB per vertex.
                                // A normal engine Solid layer owns that quad.
                                let color=[(c>>16) as u8,(c>>8) as u8,c as u8,(c>>24) as u8];
                                let payload=plane.layer+1;
                                let mut commands=Vec::new();
                                if plane.payload.is_some(){commands.push(json!({"op":"remove","id":payload}));}
                                commands.push(json!({"op":"put","layer":{"id":payload,"parent":plane.layer,"rect":[0.,0.,w,h],"paint":{"kind":"solid","color":color}}}));
                                commands.push(json!({"op":"set_depth","id":payload,"depth":0.}));
                                // 679140's new native record has Src[1]=0:
                                // resource creation itself does not show it.
                                commands.push(json!({"op":"set_visible","id":plane.layer,"visible":false}));
                                host.submit_engine(self.client,1,&serde_json::to_vec(&json!({"layers":commands}))?)?;
                                self.tasks.set_payload(id)?;
                            }
                            machine.acknowledge(None)?;
                        }
                        77 => {
                            ensure!(call.arguments.len()==1,"CatSystem2 native task query arguments");
                            machine.acknowledge(Some(u32::from(self.tasks.exists(call.arguments[0]))))?;
                        }
                        88 => {
                            ensure!(call.arguments.len()==2,"CatSystem2 plane Z arguments");
                            machine.preflight_native(false,&[])?;
                            let id=call.arguments[0];
                            if id!=0 {
                                let plane=self.tasks.plane(id)?;
                                let payload=plane.payload.context("CatSystem2 plane position has no native geometry")?;
                                let z=f32::from_bits(call.arguments[1]);
                                ensure!(z.is_finite()&&z.abs()<=1_000_000.,"CatSystem2 plane Z extent budget");
                                let mut layer:crysirror_scene::layer::LayerSpec=serde_json::from_slice::<Option<crysirror_scene::layer::LayerSpec>>(&host.query_engine(self.client,EngineQuery::Layer(payload))?)?.context("CatSystem2 engine plane geometry missing")?;
                                if layer.transform.z!=z {layer.transform.z=z;}
                                let z=layer.transform.z;
                                // 67F5B0's ordinary display pass has [0,1e6).
                                // Capture67F960/67FB80 has a distinct65536
                                // upper bound and is not implied by this map.
                                // Global native Z belongs to the payload, not
                                // the Task parent or a depth-inheriting group.
                                host.submit_engine(self.client,1,&serde_json::to_vec(&json!({"layers":[{"op":"set_transform","id":payload,"transform":layer.transform},{"op":"set_depth","id":payload,"depth":z},{"op":"set_visible","id":payload,"visible":(0.0..1_000_000.0).contains(&z)}]}))?)?;
                            }
                            machine.acknowledge(None)?;
                        }
                        92 => {
                            ensure!(call.arguments.len()==2&&call.arguments[1]<=1,"CatSystem2 plane visibility arguments/profile");
                            machine.preflight_native(false,&[])?;
                            if call.arguments[0]!=0 {
                                let mut proposed=self.tasks.clone();
                                let messages=proposed.visibility_messages(call.arguments[0],call.arguments[1]!=0)?;
                                let commands=messages.into_iter().map(|(id,visible)|json!({"op":"set_visible","id":id,"visible":visible})).collect::<Vec<_>>();
                                host.submit_engine(self.client,1,&serde_json::to_vec(&json!({"layers":commands}))?)?;
                                self.tasks=proposed;
                            }
                            machine.acknowledge(None)?;
                        }
                        71 => {
                            ensure!(call.arguments.len()==1,"CatSystem2 event wait arguments");
                            let slot=call.arguments[0];
                            //Only validates the future16-byte message and
                            //delayed boolean slot; native71 does not clear it.
                            machine.preflight_native(true,&[(slot,vec![0;16])])?;
                            machine.acknowledge(None)?;
                            self.event_wait=Some(NativeEventWait{slot,pc:call.pc});
                            *self.native_calls.entry(call.id).or_default()+=1;
                            return Ok(VmYield::BudgetExhausted);
                        }
                        104 => {
                            ensure!(call.arguments.len()==7,"CatSystem2 click controller bind arguments");
                            machine.preflight_native(false,&[])?;
                            let id=call.arguments[0];
                            let button=self.tasks.button(id)?;
                            if button.binding_target.is_none(){
                                let a=&call.arguments;let x=a[1] as i32;let y=a[2] as i32;
                                let right=x.wrapping_add(a[3] as i32);let bottom=y.wrapping_add(a[4] as i32);
                                let full_canvas=x<=0&&y<=0&&right>=self.config.canvas[0] as i32&&bottom>=self.config.canvas[1] as i32;
                                let empty=x>=right||y>=bottom;
                                let native_rect=if full_canvas{[-100000,-100000,100000,100000]}else{[x,y,right,bottom]};
                                let [left,top,right,bottom]=native_rect;
                                let rect=if empty&&!full_canvas{None}else{Some([i64::from(left),i64::from(top),i64::from(right)-i64::from(left),i64::from(bottom)-i64::from(top)])};
                                let receiver=crysirror_scene::input_routes::InputReceiverSpec{id,priority:a[5] as i32,sources:7,rect,coverage:None,enabled:true,hover_enabled:true,drag_enabled:true};
                                let mut proposed=self.tasks.clone();proposed.bind_button(id,self.tasks.current(),native_rect,a[6],full_canvas)?;
                                host.submit_engine(self.client,1,&serde_json::to_vec(&json!({"input_routes":[{"op":"put","receiver":receiver}]}))?)?;
                                self.tasks=proposed;self.input_routes_bound=true;
                            }
                            machine.acknowledge(None)?;
                        }
                        105 => {
                            ensure!(call.arguments.len()==2&&call.arguments[1]<=1,"CatSystem2 click controller enabled arguments/profile");
                            machine.preflight_native(false,&[])?;
                            let id=call.arguments[0];
                            if self.tasks.button(id)?.binding_target.is_some(){
                                host.submit_engine(self.client,1,&serde_json::to_vec(&json!({"input_routes":[{"op":"set_enabled","id":id,"enabled":call.arguments[1]!=0}]}))?)?;
                            }
                            machine.acknowledge(None)?;
                        }
                        848 => {
                            ensure!(call.arguments.len()==2&&call.arguments[1]<=1,"CatSystem2 hover enabled arguments/profile");
                            machine.preflight_native(false,&[])?;let id=call.arguments[0];
                            if self.tasks.button(id)?.binding_target.is_some(){
                                let mut receiver:crysirror_scene::input_routes::InputReceiverSpec=serde_json::from_slice::<Option<crysirror_scene::input_routes::InputReceiverSpec>>(&host.query_engine(self.client,EngineQuery::InputRouteReceiver(id))?)?.context("CatSystem2 engine input receiver missing")?;
                                receiver.hover_enabled=call.arguments[1]!=0;
                                host.submit_engine(self.client,1,&serde_json::to_vec(&json!({"input_routes":[{"op":"put","receiver":receiver}]}))?)?;
                            }
                            machine.acknowledge(None)?;
                        }
                        849 => {
                            ensure!(call.arguments.len()==2&&call.arguments[1]<=1,"CatSystem2 drag enabled arguments/profile");
                            machine.preflight_native(false,&[])?;let id=call.arguments[0];
                            if self.tasks.button(id)?.binding_target.is_some(){
                                let mut receiver:crysirror_scene::input_routes::InputReceiverSpec=serde_json::from_slice::<Option<crysirror_scene::input_routes::InputReceiverSpec>>(&host.query_engine(self.client,EngineQuery::InputRouteReceiver(id))?)?.context("CatSystem2 engine input receiver missing")?;
                                receiver.drag_enabled=call.arguments[1]!=0;
                                host.submit_engine(self.client,1,&serde_json::to_vec(&json!({"input_routes":[{"op":"put","receiver":receiver}]}))?)?;
                            }
                            machine.acknowledge(None)?;
                        }
                        256 => {
                            ensure!(call.arguments.len()==3,"CatSystem2 image-list initialization arguments");
                            machine.preflight_native(false,&[])?;let id=call.arguments[0];
                            ensure!(self.tasks.image_list(id)?.manager.is_none(),"CatSystem2 unsupported image-list reinitialization");
                            if self.image_pool_loading.is_none(){
                                self.image_pool_loading=crate::image_list::PendingImages::begin(self.pool.as_ref().context("CatSystem2 worker pool missing")?,host,self.client,id,call.pc,[call.arguments[1],call.arguments[2]],self.config.canvas)?;
                                return Ok(VmYield::BudgetExhausted);
                            }
                            let pending=self.image_pool_loading.as_mut().unwrap();
                            ensure!(pending.task_id==id&&pending.native_pc==call.pc,"CatSystem2 image-list worker ownership mismatch");
                            let Some(images)=pending.poll()? else{return Ok(VmYield::BudgetExhausted);};
                            let mut proposed=self.tasks.clone();let plane=proposed.create_plane(id,b"base plane".to_vec())?;
                            let mut group=plane.layer_spec()?;group.visible=false;
                            let mut payload:crysirror_scene::layer::LayerSpec=serde_json::from_value(json!({"id":plane.layer+1,"parent":plane.layer,"rect":[0,0,self.config.canvas[0],self.config.canvas[1]],"transform":{"z":1000.},"paint":{"kind":"solid","color":[0,0,0,0]}}))?;
                            let atlas=host.publish_image_resource(self.client,0x43000000+id*2,images.atlas.clone())?;
                            let base=match host.publish_image_resource(self.client,plane.layer+1,images.base.clone()){
                                Ok(base)=>base,Err(e)=>{host.release_image_resource(self.client,atlas.path.relative())?;return Err(e.into());}
                            };
                            payload.paint=crysirror_scene::layer::Paint::Image{asset:base.path.relative().to_owned(),tint:[255;4]};
                            let submit=host.submit_engine(self.client,1,&serde_json::to_vec(&json!({"layers":[{"op":"put","layer":group},{"op":"put","layer":payload},{"op":"set_depth","id":plane.layer+1,"depth":1000.}]}))?);
                            if let Err(e)=submit{host.release_image_resource(self.client,base.path.relative())?;host.release_image_resource(self.client,atlas.path.relative())?;return Err(e.into());}
                            let size=images.atlas.size();let page=images.page_size;
                            proposed.bind_image_pool(id,crate::task_policy::NativeImagePool{resource:images.atlas,path:atlas.path.relative().to_owned(),size,page_size:page,page_count:size[0].div_ceil(page[0])*size[1].div_ceil(page[1]),dirty_bounds:None,upload_position:None,native_entry_keys:Default::default()},plane.id,crate::task_policy::NativePlaneImage{resource:images.base,path:base.path.relative().to_owned()})?;
                            self.tasks=proposed;self.image_pool_loading=None;machine.acknowledge(None)?;
                        }
                        260 => {
                            ensure!(call.arguments.len()==3,"CatSystem2 image-list upload position arguments");
                            machine.preflight_native(false,&[])?;
                            self.tasks.set_image_upload_position(call.arguments[0],[call.arguments[1] as i32,call.arguments[2] as i32])?;
                            machine.acknowledge(None)?;
                        }
                        262 => {
                            ensure!(call.arguments.len()==2&&call.arguments[1]<=1,"CatSystem2 image-list presentation mode arguments/profile");
                            machine.preflight_native(false,&[])?;
                            self.tasks.set_image_mode(call.arguments[0],call.arguments[1])?;
                            machine.acknowledge(None)?;
                        }
                        449 => {
                            ensure!(call.arguments.len()==2,"CatSystem2 FES image-list binding arguments");
                            machine.preflight_native(false,&[])?;
                            self.tasks.bind_fes_image_list(call.arguments[0],call.arguments[1])?;
                            machine.acknowledge(None)?;
                        }
                        247 => {
                            ensure!(call.arguments.len()==1,"CatSystem2 CSS load arguments");
                            machine.preflight_native(true,&[])?;
                            ensure!(self.saves.len()<256,"CatSystem2 CSS handle budget");
                            if self.save_loading.is_none(){
                                let permit=match self.pool.as_ref().context("CatSystem2 worker pool missing")?.reserve(){Ok(p)=>p,Err(e) if e.is::<crysirror_tasks::CapacityExceeded>()=>return Ok(VmYield::BudgetExhausted),Err(e)=>return Err(e)};
                                let name=crate::native_values::format_bytes(machine,call.arguments[0])?;
                                let name=std::str::from_utf8(&name).context("CatSystem2 unsupported save filename encoding")?.to_owned();
                                ensure!(name.len()<260,"CatSystem2 save filename extent");
                                let storage=host.storage(self.client)?;let client=self.client;
                                self.save_loading=Some(permit.spawn(move |cancel|crate::native_save::load(storage.as_ref(),client,&name,cancel))?);
                                return Ok(VmYield::BudgetExhausted);
                            }
                            let Some(saved)=self.save_loading.as_mut().unwrap().try_take()? else{return Ok(VmYield::BudgetExhausted);};
                            let id=if let Some(saved)=saved{let id=self.next_save;self.next_save=id.checked_add(1).context("CatSystem2 CSS handle overflow")?;self.saves.insert(id,saved);id}else{0};
                            machine.acknowledge(Some(id))?;self.save_loading=None;
                        }
                        276 => {
                            ensure!(call.arguments.len()==2,"CatSystem2 string variable assignment arguments");
                            machine.preflight_native(false,&[])?;
                            let bytes=machine.read_cstring(call.arguments[1],65534)?;
                            self.string_variables.assign(call.arguments[0],bytes)?;
                            machine.acknowledge(None)?;
                        }
                        277 => {
                            ensure!(call.arguments.len()==2,"CatSystem2 string variable query arguments");
                            let value=self.string_variables.get(call.arguments[0]);
                            let mut bytes=value.unwrap_or(b"").to_vec();bytes.push(0);
                            machine.commit_native(Some(u32::from(value.is_some())),&[(call.arguments[1],bytes)])?;
                        }
                        938 => {
                            ensure!(call.arguments.is_empty(),"CatSystem2 sound config found arguments");
                            machine.preflight_native(true,&[])?;
                            if self.sound_definition.is_none() {
                                if self.sound_loading.is_none() {
                                    let permit=match self.pool.as_ref().context("CatSystem2 worker pool missing")?.reserve() {Ok(p)=>p,Err(e) if e.is::<crysirror_tasks::CapacityExceeded>()=>return Ok(VmYield::BudgetExhausted),Err(e)=>return Err(e)};
                                    let storage=host.storage(self.client)?;
                                    self.sound_loading=Some(permit.spawn(move |cancel|crate::sound_definition::load(storage.as_ref(),&||cancel.check()))?);
                                    return Ok(VmYield::BudgetExhausted);
                                }
                                let Some(definition)=self.sound_loading.as_mut().unwrap().try_take()? else{return Ok(VmYield::BudgetExhausted);};
                                self.sound_definition=Some(definition);self.sound_loading=None;
                            }
                            machine.acknowledge(Some(self.sound_definition.as_ref().unwrap().found()))?;
                        }
                        209 => {
                            ensure!(call.arguments.len()==1,"CatSystem2 interpreter constructor arguments");
                            machine.preflight_native(false,&[])?;
                            if self.interpreter.is_none() {
                                self.interpreter=Some(crate::native_interpreter::Interpreter::new(call.arguments[0])?);
                            }
                            machine.acknowledge(None)?;
                        }
                        210 => {
                            ensure!(call.arguments.len()==1,"CatSystem2 flag query arguments");
                            machine.acknowledge(Some(self.interpreter.as_ref().map(|i|i.flag(call.arguments[0])).unwrap_or(0)))?;
                        }
                        211|212 => {
                            ensure!(call.arguments.len()==if call.id==211{2}else{1},"CatSystem2 interpreter mutation arguments");
                            machine.preflight_native(false,&[])?;
                            if let Some(interpreter)=self.interpreter.as_mut() {
                                if call.id==211 {interpreter.set_flag(call.arguments[0],call.arguments[1]);}
                                else {interpreter.set_default(call.arguments[0]);}
                            }
                            machine.acknowledge(None)?;
                        }
                        213 => {
                            ensure!(call.arguments.len()==1,"CatSystem2 expression arguments");
                            machine.preflight_native(true,&[])?;
                            let value=if let Some(interpreter)=self.interpreter.as_mut() {
                                let mut source=machine.read_cstring(call.arguments[0],2047)?;
                                if source.last()==Some(&b'%'){source.pop();}
                                interpreter.evaluate(&source)?
                            }else{0};
                            machine.acknowledge(Some(value))?;
                        }
                        359|789 => {
                            ensure!(if call.id==359 {call.arguments.len()==4}else{call.arguments.is_empty()},"CatSystem2 native settings arguments");
                            machine.preflight_native(call.id==789,&[])?;
                            if self.settings.is_none() {
                                if self.settings_loading.is_none() {
                                    let permit=match self.pool.as_ref().context("CatSystem2 worker pool missing")?.reserve() {Ok(p)=>p,Err(e) if e.is::<crysirror_tasks::CapacityExceeded>()=>return Ok(VmYield::BudgetExhausted),Err(e)=>return Err(e)};
                                    let storage=host.storage(self.client)?;
                                    self.settings_loading=Some(permit.spawn(move |cancel|crate::native_settings::load(storage.as_ref(),&||cancel.check()))?);
                                    return Ok(VmYield::BudgetExhausted);
                                }
                                let Some(settings)=self.settings_loading.as_mut().unwrap().try_take()? else{return Ok(VmYield::BudgetExhausted);};
                                if call.id==359 {machine.commit_native(None,&settings.writes(&call.arguments)?)?;}else{machine.acknowledge(Some(settings.flag_count))?;}
                                self.settings=Some(settings);self.settings_loading=None;
                            } else {
                                let settings=self.settings.as_ref().unwrap();
                                if call.id==359 {machine.commit_native(None,&settings.writes(&call.arguments)?)?;}else{machine.acknowledge(Some(settings.flag_count))?;}
                            }
                        }
                        400 => {
                            ensure!(call.arguments.len()==2,"CatSystem2 LoopPoint import arguments");
                            machine.preflight_native(false,&[])?;
                            if self.loop_loading.is_none() {
                                let permit=match self.pool.as_ref().context("CatSystem2 worker pool missing")?.reserve() {
                                    Ok(p)=>p,
                                    Err(e) if e.is::<crysirror_tasks::CapacityExceeded>()=>return Ok(VmYield::BudgetExhausted),
                                    Err(e)=>return Err(e),
                                };
                                let name=crate::native_values::format(machine,call.arguments[1])?.replace('\\',"/");
                                let path=VfsPath::new(Mount::Asset,&name)?;ensure!(path.relative()==name,"CatSystem2 LoopPoint capability path");
                                let storage=host.storage(self.client)?;
                                self.loop_loading=Some(permit.spawn(move |cancel| {
                                    cancel.check()?;
                                    let reader=match storage.open_asset(&path) {Ok(r)=>r,Err(Error::NotFound)=>return Ok(PreparedLoops{entries:None,source:name}),Err(e)=>return Err(e.into())};
                                    let bytes=crysirror_resource::read_all(reader.as_ref(),crate::loop_policy::MAX_LOOP_FILE as u64)?;
                                    cancel.check()?;
                                    let startup=crysirror_resource::read_all(storage.open_asset(&VfsPath::new(Mount::Asset,"config/startup.xml")?)?.as_ref(),1024*1024)?;
                                    let sound_path=crate::loop_policy::SoundPaths::config_name(&startup)?;
                                    let sound=crysirror_resource::read_all(storage.open_asset(&VfsPath::new(Mount::Asset,&sound_path)?)?.as_ref(),1024*1024)?;
                                    cancel.check()?;
                                    let paths=crate::loop_policy::SoundPaths::parse(&startup,&sound)?;
                                    let entries=crate::loop_policy::parse_checked(&bytes,&paths,&||cancel.check())?;
                                    cancel.check()?;Ok(PreparedLoops{entries:Some(entries),source:name})
                                })?);
                                return Ok(VmYield::BudgetExhausted);
                            }
                            let Some(prepared)=self.loop_loading.as_mut().unwrap().try_take()? else{return Ok(VmYield::BudgetExhausted);};
                            if let Some(mut entries)=prepared.entries {
                                for entry in &mut entries {
                                    if let Some(previous)=host.audio_loop_metadata(self.client,&entry.asset)? {
                                        ensure!(previous.range.timebase==entry.range.timebase,"CatSystem2 loop timebase changed");
                                        entry.range.end_frame=previous.range.end_frame;
                                    }
                                }
                                let receipt=host.publish_audio_loop_metadata(self.client,&entries)?;
                                ensure!(receipt.client==self.client&&receipt.entries as usize>=entries.len(),"CatSystem2 loop catalog owner receipt");
                                self.loop_catalog_revision=receipt.revision;self.loop_entries=receipt.entries;
                            } else {
                                host.append_log(self.client,&format!("Can't find LoopPointFile : {}",prepared.source))?;
                            }
                            machine.acknowledge(None)?;self.loop_loading=None;
                        }
                        590 => {
                            ensure!(call.arguments.len()==1,"CatSystem2 XML load arguments");
                            machine.preflight_native(true,&[])?;
                            if self.xml_loading.is_none() {
                                ensure!(self.documents.len()<64,"CatSystem2 XML handle budget");
                                let permit = match self.pool.as_ref().context("CatSystem2 worker pool missing")?.reserve() {
                                    Ok(p)=>p,
                                    Err(e) if e.is::<crysirror_tasks::CapacityExceeded>()=>return Ok(VmYield::BudgetExhausted),
                                    Err(e)=>return Err(e),
                                };
                                let name=crate::native_values::format(machine,call.arguments[0])?;
                                let path=VfsPath::new(Mount::Asset,&name)?;
                                ensure!(path.relative()==name,"CatSystem2 XML capability path");
                                let storage=host.storage(self.client)?;
                                self.xml_loading=Some(permit.spawn(move |cancel| {
                                    cancel.check()?;
                                    let reader=match storage.open_asset(&path) {
                                        Ok(r)=>r,
                                        Err(Error::NotFound)=>return Ok(None),
                                        Err(e)=>return Err(e.into()),
                                    };
                                    let bytes=crysirror_resource::read_all(reader.as_ref(),crysirror_resource::xml::MAX_XML_BYTES as u64)?;
                                    cancel.check()?;
                                    let document=XmlDocument::parse(&bytes)?;
                                    cancel.check()?;
                                    Ok(Some(Arc::new(document)))
                                })?);
                                return Ok(VmYield::BudgetExhausted);
                            }
                            let Some(document)=self.xml_loading.as_mut().unwrap().try_take()? else {return Ok(VmYield::BudgetExhausted)};
                            self.xml_loading=None;
                            if let Some(document)=document {
                                let bytes=self.document_bytes.checked_add(document.resident_bytes()).context("CatSystem2 XML resident overflow")?;
                                ensure!(bytes<=32*1024*1024,"CatSystem2 XML resident budget");
                                let id=self.next_document;
                                let next=id.checked_add(1).context("CatSystem2 XML handle exhaustion")?;
                                // Owner publication precedes returning the real logical handle.
                                self.documents.insert(id,document);
                                self.document_bytes=bytes;
                                self.next_document=next;
                                machine.acknowledge(Some(id))?;
                            } else { machine.acknowledge(Some(0))?; }
                        }
                        591 => {
                            ensure!(call.arguments.len()==1,"CatSystem2 XML close arguments");
                            let id=call.arguments[0];
                            if id!=0 {ensure!(self.documents.contains_key(&id),"CatSystem2 invalid XML handle {id}");}
                            machine.preflight_native(false,&[])?;
                            if let Some(doc)=self.documents.remove(&id) {self.document_bytes-=doc.resident_bytes();}
                            machine.acknowledge(None)?;
                        }
                        592|593 => {
                            ensure!(call.arguments.len()==3,"CatSystem2 XML query arguments");
                            let id=call.arguments[0];
                            let node=if id==0 {None} else {
                                let document=self.documents.get(&id).with_context(||format!("CatSystem2 invalid XML handle {id}"))?;
                                let path=crate::native_values::format(machine,call.arguments[2])?;
                                xml_node(document,&path)?
                            };
                            if call.id==592 {
                                let value=node.map(|n|crate::native_values::integer(&n.text)).unwrap_or(call.arguments[1]);
                                machine.acknowledge(Some(value))?;
                            } else if let Some(node)=node {
                                ensure!(node.text.len()<65536,"CatSystem2 XML string result budget");
                                let mut bytes=node.text.as_bytes().to_vec(); bytes.push(0);
                                machine.commit_native(None,&[(call.arguments[1],bytes)])?;
                            } else {machine.acknowledge(None)?;}
                        }
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
                        13|14|15|16 => {
                            ensure!(call.arguments.len()==2,"CatSystem2 native string arguments");
                            let mut bytes=if call.id==13||call.id==15 {machine.read_cstring(call.arguments[1],65534)?} else {crate::native_values::format_bytes(machine,call.arguments[1])?};
                            if call.id==13||call.id==15 {
                                let destination=u64::from(call.arguments[0]);
                                let source=u64::from(call.arguments[1]);
                                let old=if call.id==15 {machine.read_cstring(call.arguments[0],65534)?.len()}else{0};
                                let overlaps=destination<source+bytes.len() as u64+1&&source<destination+old as u64+bytes.len() as u64+2;
                                ensure!(!overlaps||(call.id==13&&destination==source),"unsupported overlapping native CatSystem2 string operation");
                            }
                            if call.id==15||call.id==16 {
                                let mut prefix=machine.read_cstring(call.arguments[0],65534)?;
                                ensure!(prefix.len()+bytes.len()<=65534,"CatSystem2 native append length budget");
                                prefix.extend(bytes);bytes=prefix;
                            }
                            // Original native copy/append terminate with a WORD zero.
                            bytes.extend([0,0]);
                            machine.commit_native(Some(call.arguments[0]),&[(call.arguments[0],bytes)])?;
                        }
                        17|18 => {
                            ensure!(call.arguments.len()==1,"CatSystem2 native string length arguments");
                            let bytes=if call.id==17 {machine.read_cstring(call.arguments[0],65534)?}else{crate::native_values::format_bytes(machine,call.arguments[0])?};
                            machine.acknowledge(Some(bytes.len() as u32))?;
                        }
                        393=>{
                            ensure!(call.arguments.len()==1&&self.menu_installed,"CatSystem2 GMenuReturnTitle argument/menu boundary");
                            machine.preflight_native(false,&[])?;
                            host.submit_engine(self.client,1,&serde_json::to_vec(&json!({"ui":[{"op":"set_menu_enabled","id":crate::native_menu::PRODUCTION_MENU,"item":crate::native_menu::RETURN_TITLE,"enabled":call.arguments[0]!=0}]}))?)?;
                            machine.acknowledge(None)?;
                        }
                        465=>{
                            ensure!(call.arguments.is_empty()&&self.debug_key_absent,"CatSystem2 debug query requires verified absent native debug key");
                            machine.acknowledge(Some(0))?;
                        }
                        _=>anyhow::bail!("CatSystem2 unsupported native gcall {} at {} PC {:#x} ({} argument bytes); original call remains unacknowledged",call.id,self.config.script,call.pc,call.arguments.len()*4),
                    }
                    *self.native_calls.entry(call.id).or_default() += 1;
                    if remaining == 0 {
                        return Ok(VmYield::BudgetExhausted);
                    }
                }
            }
        }
    }
}
// Original XML treats a single <document> root as an invisible wrapper.
fn xml_node<'a>(
    document: &'a XmlDocument,
    path: &str,
) -> Result<Option<&'a crysirror_resource::xml::XmlNode>> {
    let parts = path
        .split('/')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return Ok(None);
    }
    if document.root().name == "document" {
        return document.child_path(path);
    }
    if parts[0] != document.root().name {
        return Ok(None);
    }
    document.child_path(&parts[1..].join("/"))
}
impl VmAdapter for CatSystem2Vm {
    fn preparation_pending(&self) -> bool {
        self.program.is_none()
    }
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
        serde_json::to_vec(&json!({"profile":"kcs-v2","script":self.config.script,"loaded":self.program.is_some(),"pc":self.machine.as_ref().map(|m|m.pc),"instructions":self.machine.as_ref().map(|m|m.instructions),"pending_native":self.machine.as_ref().and_then(|m|m.pending.as_ref()),"native_calls":self.native_calls,"tasks":self.tasks.diagnostics(),"image_pool_pending":self.image_pool_loading.is_some(),"native_event_wait":self.event_wait.as_ref().map(|w|json!({"slot":w.slot,"native_pc":w.pc,"resume":"next_owner_tick_zero_or_message_one"})),"input_routes_bound":self.input_routes_bound,"string_variables":self.string_variables.diagnostics(),"save_containers":self.saves.iter().map(|(id,c)|json!({"id":id,"container":c.diagnostics()})).collect::<Vec<_>>(),"save_container_pending":self.save_loading.is_some(),"sound_definition":self.sound_definition.as_ref().map(|s|s.diagnostics()),"sound_definition_pending":self.sound_loading.is_some(),"interpreter":self.interpreter.as_ref().map(|i|i.diagnostics()),"pending_native_text":self.machine.as_ref().and_then(|m|m.pending.as_ref().and_then(|p|match p.id{66=>p.arguments.get(2),447=>p.arguments.get(1),_=>None}).and_then(|a|crate::native_values::format(m,*a).ok())),"pending_resource":self.machine.as_ref().and_then(|m|m.pending.as_ref().filter(|p|p.id==400).and_then(|p|p.arguments.get(1)).and_then(|address|crate::native_values::format(m,*address).ok())),"xml_documents":self.documents.len(),"xml_resident_bytes":self.document_bytes,"loop_catalog_revision":self.loop_catalog_revision,"loop_catalog_entries":self.loop_entries,"loop_import_pending":self.loop_loading.is_some(),"settings_source":self.settings.as_ref().map(|s|json!({"policy":"client_save_setup_xml","windows_imported":false,"gamedef_loaded":s.gamedef,"setup_loaded":s.setup,"flag_count":s.flag_count})),"settings_pending":self.settings_loading.is_some(),"error":self.error})).unwrap_or_default()
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

#[cfg(test)]
mod tests {
    #[test]
    fn original_xml_document_wrapper_and_defaults_use_real_nodes() {
        let d = crysirror_resource::xml::XmlDocument::parse(
            b"<document><x><z/></x><x><y>23</y></x></document>",
        )
        .unwrap();
        assert_eq!(super::xml_node(&d, "/x//y/").unwrap().unwrap().text, "23");
        assert!(super::xml_node(&d, "document/x/y").unwrap().is_none());
        assert!(super::xml_node(&d, "").unwrap().is_none());
        let d = crysirror_resource::xml::XmlDocument::parse(b"<r><x>7</x></r>").unwrap();
        assert_eq!(super::xml_node(&d, "r/x").unwrap().unwrap().text, "7");
        assert!(super::xml_node(&d, "x").unwrap().is_none());
    }
}
