//! Bounded native offset2 MES interpreter. Addresses in comments refer to the
//! owner's DC4PH executable, analysed statically; no executable code is hosted.
//! UI records here are native declarations, not a layout/hit-test/compositor.
use crate::{Arguments, Instruction, Profile, Program};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImagePart {
    pub position: [i32; 2],
    pub source: [i16; 4],
    pub slot: u8,
    pub state: i16,
    pub auxiliary_flag: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Button {
    pub bounds: [i16; 4],
    pub action: String,
    pub sounds: [u8; 2],
    /// Native current-record flags copied with the button, not the whole group.
    #[serde(default)]
    pub behavior_flags:BTreeSet<u8>,
    pub images: Vec<ImagePart>,
    pub texts: Vec<TextPart>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextPart {
    pub position: [i16; 2],
    pub text: String,
    pub color: i16,
    pub size: i16,
    pub state: i16,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Group {
    pub buttons: Vec<Button>,
    pub images: Vec<ImagePart>,
    pub texts: Vec<TextPart>,
    pub behavior_flags: BTreeSet<u8>,
    /// Native 0x7b image animation declarations. Evaluated only when activated.
    pub image_animations: Vec<[i16; 4]>,
    /// Original group-local translation. Native78 declares, not displays it.
    pub position: [i16; 2],
    /// Original412B50 pending20-render-tick movement declaration. Activation
    /// must map this to a shared timeline before presenting the group.
    pub motion: Option<GroupMotion>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupMotion {
    pub from: [i16; 2],
    pub to: [i16; 2],
    pub duration_ticks: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MusicAsset {
    pub name: String,
    pub loop_start: u64,
    pub loop_end: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SystemReload {
    /// Both original preference blocks were read and validated.
    Loaded(crate::settings::SystemData),
    /// Native40EB40 calls40E9C0(reset_all=1) if the file is absent.
    Missing,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MixerControls {
    pub enabled: [bool; 3],
    pub gain: [f32; 3],
}
#[derive(Clone, Debug)]
pub enum Event {
    /// Original scene-wide falling effect selector and encoded density/wind.
    FallingEffect {
        kind: u8,
        parameter: u8,
    },
    PlayMusic {
        asset: MusicAsset,
        looping: bool,
    },
    /// Built-in native name/mail input, completed through engine-owned UI.
    PlayerInput,
    /// Finalize the original central group and enter process11.
    BeginMenu(Group),
    Cursor(String),
    LoadImage {
        slot: u8,
        name: String,
    },
    DefineGroup {
        slot: u8,
        group: Group,
    },
    GroupVisibility(bool),
    /// Original49/425C20 ends retained transitions and current UI groups.
    ResetScene,
    DisplayImage {
        slot: u8,
    },
    Palette {
        index: u8,
        rgb: [u8; 3],
    },
    SpritePosition {
        slot: u8,
        position: [i16; 2],
    },
    /// Native read labels are VM/save policy, not visual labels.
    ReadLabel {
        scene: String,
        label: u16,
    },
    PlayAudio {
        channel: u32,
        name: String,
        voice: bool,
    },
    StopAudio {
        channel: u32,
    },
    BeginFade {
        out: bool,
        duration_ms: u64,
        color: [u8; 3],
    },
}
/// Resource preparation may complete on a worker; `false` retains the current
/// instruction and yields without inventing success or advancing native PC.
pub trait NativeHost {
    /// A real engine UI activation, or None while process11 remains waiting.
    fn menu_action(&mut self)->Result<Option<String>>{bail!("Circus native menu interaction unavailable")}

    fn reload_system(&mut self) -> Result<Option<SystemReload>> {
        bail!("Circus native system.dat reload unavailable")
    }
    fn save_readable(&mut self, _slot: u16) -> Result<Option<bool>> {
        bail!("Circus native save availability probe unavailable")
    }
    fn random(&mut self, _modulus: u16) -> Result<Option<i32>> {
        bail!("Circus native shared random sequence unavailable")
    }
    fn music(&mut self, _id: u16, _looping: bool) -> Result<Option<MusicAsset>> {
        bail!("Circus native music table unavailable")
    }
    fn scene(&mut self, name: &str) -> Result<Option<Arc<Program>>>;
    fn emit(&mut self, event: &Event) -> Result<bool>;
    fn save_exists(&mut self, slot: u16) -> Result<bool>;
    fn sound_name(&mut self, _id: u16) -> Result<Option<String>> {
        bail!("Circus native sound ID table unavailable")
    }
    fn fade_complete(&mut self, _out: bool) -> Result<bool> {
        bail!("Circus native fade completion unavailable")
    }
    fn audio_playing(&mut self, _channel: u32) -> Result<bool> {
        bail!("Circus native audio status unavailable")
    }
    fn write_system(&mut self, _bytes: &[u8]) -> Result<bool> {
        bail!("Circus native system.dat persistence unavailable")
    }
    fn scene_index(&mut self, _name: &str) -> Result<Option<i32>> {
        bail!("Circus native scene index unavailable")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Advanced,
    Pending,
    WaitUntil(u64),
    WaitForInput,
    Halted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct State {
    pub scene: String,
    pub pc: u32,
    pub instructions: u64,
    pub version: String,
    /// Byte banks and indexed system values retain native value semantics.
    bytes: BTreeMap<u16, u8>,
    system: BTreeMap<u8, i32>,
    pub system_data: crate::settings::SystemData,
    pending_system_save: bool,
    /// Completed reload data retained while dependent music metadata is pending.
    pending_system_reload: Option<SystemReload>,
    pub image_names: BTreeMap<u8, String>,
    pub groups: BTreeMap<u8, Group>,
    pending_images: BTreeSet<u8>,
    builder: Group,
    current_button: Option<Button>,
    read: BTreeSet<(String, u16)>,
    /// Native4036D0 current contiguous read-label record (no host pointer).
    read_run: Option<(String, u16, u16)>,
    wait_until: Option<u64>,
    wait_click: bool,
    #[serde(default)]
    menu_wait:bool,
    wait_skippable: bool,
    pub halted: bool,
    pub savepoint: Option<(String, u16, u32, u16)>,
    pub voice_prefixes: Vec<String>,
    pub video_backend: Option<i16>,
    /// Native scene field1764 is zeroed by the ctor until an effect opcode
    /// explicitly creates a colour texture. This is semantic data, no pointer.
    pub transition_color: Option<[u8; 3]>,
    pub native_flags: BTreeMap<u32, i32>,
    /// Protagonist names are native script state, distinct from CLEARNAME/mail.
    pub protagonist_names: [String; 2],
    /// Native system preference names at184/200. Runtime names remain empty
    /// until opcode11 copies these buffers (ctor40EC50/40E9C0).
    pub configured_names: [String; 2],
    /// Native 0x51 animation resource prefix (scene member11224).
    pub animation_prefix: String,
    /// Native callback label buffers keyed by scene-member byte offset.
    pub callbacks: BTreeMap<u32, String>,
    builder_offset: [i16; 2],
    transition_wait: Option<bool>,
    transition_poll_at: Option<u64>,
    wait_animations: bool,
    wait_audio_mask: u8,
    pending_audio: Vec<(u32, String, bool)>,
    current_music: i16,
    pending_music: Option<(MusicAsset, bool)>,
    /// Native filename buffer21944, retained across category disable/reset.
    last_music: Option<MusicAsset>,
    /// Original402320 records at most200 random results for save/replay policy.
    random_history: Vec<i32>,
}
impl State {
    pub(crate) fn audio_controls(&self) -> Result<MixerControls> {
        let master = self.value(99, 225)?;
        ensure!(
            (0..=100).contains(&master),
            "Circus master volume outside native range"
        );
        let mut gain = [0.; 3];
        let enabled = [
            self.value(99, 14)? != 0,
            self.value(99, 15)? != 0,
            self.value(99, 5)? != 0,
        ];
        for (index, selector) in [7, 8, 9].into_iter().enumerate() {
            let volume = self.value(99, selector)?;
            ensure!(
                (0..=100).contains(&volume),
                "Circus bus volume outside native range"
            );
            if master != 0 && volume != 0 && self.value(99, 203)? != 0 {
                let centibels = 5000 * (volume * master / 100) / 100 - 5000;
                gain[index] = 10f32.powf(centibels as f32 / 2000.);
            }
        }
        Ok(MixerControls { enabled, gain })
    }
    pub fn new(scene: String) -> Self {
        // sub_40E7D0 + initialized .data, getter sub_40CC50. These are native
        // fresh-install defaults, not guessed fallback values for unknown vars.
        let system = [
            (1, 0),
            (5, 1),
            (7, 60),
            (8, 70),
            (9, 100),
            (10, 1),
            (11, 49),
            (12, 6),
            (13, 0),
            (14, 1),
            (15, 1),
            (16, 0),
            (17, 0),
            (18, 0),
            (19, 0),
            (23, 0),
            (24, 0),
            (25, 0),
            (26, 3),
            (27, 0),
            (28, 1),
            (29, 3),
            (30, 1),
            (180, 1),
            (181, 1),
            (190, 0),
            (191, 0),
            (194, 5),
            (196, 4),
            (197, 2),
            (198, 5),
            (199, 0),
            (200, 1),
            (201, 1),
            (202, 0),
            (203, 1),
            (205, 1),
            (206, 80),
            (207, 80),
            (208, 0),
            (209, 0),
            (210, 0),
            (211, 1),
            (212, 1),
            (213, 1),
            (237, 1),
        ]
        .into_iter()
        .collect();
        Self {
            scene,
            pc: 3,
            instructions: 0,
            version: String::new(),
            bytes: BTreeMap::new(),
            system,
            system_data: crate::settings::SystemData::default(),
            pending_system_save: false,
            pending_system_reload: None,
            image_names: BTreeMap::new(),
            groups: BTreeMap::new(),
            pending_images: BTreeSet::new(),
            builder: Group::default(),
            current_button: None,
            read: BTreeSet::new(),
            read_run: None,
            wait_until: None,
            wait_click: false,
            menu_wait:false,
            wait_skippable: false,
            halted: false,
            savepoint: None,
            voice_prefixes: Vec::new(),
            video_backend: None,
            transition_color: None,
            native_flags: BTreeMap::new(),
            protagonist_names: [String::new(), String::new()], // sub_40EC50 zeroes these native name buffers.
            configured_names: ["常坂".into(), "一登".into()],
            animation_prefix: String::new(),
            callbacks: BTreeMap::new(),
            builder_offset: [0; 2],
            transition_wait: None,
            transition_poll_at: None,
            wait_animations: false,
            wait_audio_mask: 0,
            pending_audio: Vec::new(),
            current_music: 0,
            pending_music: None,
            last_music: None,
            random_history: Vec::new(),
        }
    }
    pub fn value(&self, bank: i16, index: i16) -> Result<i32> {
        if bank == 99 {
            if let Some(value) = self.system_data.value(u8::try_from(index)?)? {
                return Ok(value);
            }
            return self
                .system
                .get(&(u8::try_from(index)?))
                .copied()
                .with_context(|| format!("unimplemented Circus system selector {index}"));
        }
        if (0..=10).contains(&bank) && (0..100).contains(&index) {
            return Ok(i32::from(
                *self
                    .bytes
                    .get(&((bank as u16) * 100 + index as u16))
                    .unwrap_or(&0),
            ));
        }
        if let Some(base) = byte_bank(bank, index) {
            return Ok(i32::from(
                self.system_data
                    .byte(usize::from(base + index as u16) - 1832),
            ));
        }
        bail!("unimplemented Circus value bank/index {bank}/{index}")
    }
    fn assign(&mut self, bank: i16, index: i16, value: i32) -> Result<()> {
        if bank == 99 {
            let value = if index == 225 {
                value.min(100)
            } else if index == 224 {
                i32::from(value > 0)
            } else {
                value
            };
            if matches!(index,1|23..=25|223|234..=236|242|244|255) {
                return Ok(());
            }
            ensure!(
                matches!(index,0|2|5|7..=19|26..=30|100|179..=182|183..=214|217|218|221|224..=226|233|237|241|243|245|246|253|254),
                "Circus system setter selector {index}"
            );
            let value = if value == 255 && matches!(index, 254 | 253 | 246 | 245 | 243) {
                -1
            } else {
                value
            };
            self.system.insert(u8::try_from(index)?, value);
            self.system_data.assign(u8::try_from(index)?, value)?;
            if index == 10 {
                self.system.insert(214, i32::from(value == 2));
                self.system_data.assign(214, i32::from(value == 2))?;
            }
            return Ok(());
        }
        ensure!(
            (0..=10).contains(&bank) && (0..100).contains(&index)
                || byte_bank(bank, index).is_some(),
            "Circus assignment bank/index {bank}/{index}"
        );
        let key = if bank <= 10 {
            bank as u16 * 100 + index as u16
        } else {
            byte_bank(bank, index).unwrap() + index as u16
        };
        self.bytes.insert(key, value as u8);
        if bank >= 96 {
            if bank == 100 && index == 49 {
                self.assign(99, 11, value)?;
            }
            self.system_data
                .set_byte(usize::from(key) - 1832, value as u8);
            if bank == 100 {
                self.pending_system_save = true;
            }
        }
        Ok(())
    }
    fn finish_button(&mut self) {
        if let Some(button) = self.current_button.take() {
            self.builder.buttons.push(button);
        }
    }
}
pub struct Machine {
    pub state: State,
    program: Option<Arc<Program>>,
}
impl Machine {
    /// The original render loop resumes only after synchronous native I/O
    /// queued before a wait has finished. Instruction-budget yielding is not
    /// an original rendered frame.
    pub(crate) fn native_frame_ready(&self, outcome: Outcome) -> bool {
        self.state.pending_audio.is_empty()
            && self.state.pending_music.is_none()
            && !self.state.pending_system_save
            && (matches!(outcome, Outcome::WaitUntil(_) | Outcome::WaitForInput)
                || outcome == Outcome::Pending
                    && (self.state.wait_animations || self.state.wait_audio_mask != 0))
    }
    pub fn new(scene: impl Into<String>) -> Self {
        Self {
            state: State::new(scene.into()),
            program: None,
        }
    }
    pub fn restore(state: State) -> Self {
        Self {
            state,
            program: None,
        }
    }
    pub fn step(&mut self, host: &mut dyn NativeHost, now: u64, pressed: bool) -> Result<Outcome> {
        if self.state.pending_system_save {
            if !host.write_system(self.state.system_data.bytes())? {
                return Ok(Outcome::Pending);
            }
            self.state.pending_system_save = false;
        }
        if self.state.menu_wait || self.state.wait_click
            || self.state.wait_until.is_some()
            || self.state.halted
            || self.state.wait_animations
            || self.state.wait_audio_mask != 0
        {
            if let Some((asset, looping)) = &self.state.pending_music {
                if !host.emit(&Event::PlayMusic {
                    asset: asset.clone(),
                    looping: *looping,
                })? {
                    return Ok(Outcome::Pending);
                }
                self.state.pending_music = None;
            }
            while let Some((channel, name, voice)) = self.state.pending_audio.first() {
                if !host.emit(&Event::PlayAudio {
                    channel: *channel,
                    name: name.clone(),
                    voice: *voice,
                })? {
                    return Ok(Outcome::Pending);
                }
                self.state.pending_audio.remove(0);
            }
        }
        if let Some(out) = self.state.transition_wait {
            if self.state.transition_poll_at != Some(now) {
                if host.fade_complete(out)? {
                    self.state.transition_wait = None;
                    if out {
                        self.state
                            .image_names
                            .retain(|slot, _| (37..=44).contains(slot));
                        self.state.pending_images.clear();
                    }
                }
                self.state.transition_poll_at = Some(now);
            }
        }
        if self.state.wait_animations {
            if self.state.transition_wait.is_some() {
                return Ok(Outcome::Pending);
            }
            self.state.wait_animations = false;
        }
        if self.state.wait_audio_mask != 0 {
            if host.audio_playing(8)? {
                return Ok(Outcome::Pending);
            }
            self.state.wait_audio_mask = 0;
        }
        if self.state.halted {
            return Ok(Outcome::Halted);
        }
        if self.state.menu_wait {
            let action=host.menu_action()?.or_else(||self.state.callbacks.get(&10520).filter(|s|!s.is_empty()&&*s!="-").cloned());
            let Some(action)=action else{return Ok(Outcome::WaitForInput)};
            ensure!(!action.is_empty()&&action!="-","Circus menu activation lacks native action");
            let program=self.program.as_ref().context("Circus menu program missing")?;
            let target=program.instructions.iter().find(|candidate|candidate.opcode==0x4b&&matches!(&candidate.arguments,Arguments::String(label) if label==&action)).context("Circus activated menu label missing")?;
            self.state.pc=target.offset;self.state.menu_wait=false;
        }
        if self.state.wait_click {
            if !pressed {
                return Ok(Outcome::WaitForInput);
            }
            self.state.wait_click = false;
        }
        if let Some(until) = self.state.wait_until {
            if now < until && !(pressed && self.state.wait_skippable) {
                return Ok(Outcome::WaitUntil(until));
            }
            self.state.wait_until = None;
        }
        if self.program.is_none() {
            self.program = host.scene(&self.state.scene)?;
            if self.program.is_none() {
                return Ok(Outcome::Pending);
            }
        }
        let program = self.program.as_ref().unwrap().clone();
        ensure!(
            program.profile == Profile::Dc4PlusHarmony,
            "Circus executor currently requires native ABI 0xabb6"
        );
        let i = program.instruction(self.state.pc)?.clone();
        self.execute(&program, &i, host, now).with_context(|| {
            format!(
                "Circus {} pc={:#x} opcode={:#04x}",
                self.state.scene, i.offset, i.opcode
            )
        })
    }
    fn execute(
        &mut self,
        program: &Program,
        i: &Instruction,
        host: &mut dyn NativeHost,
        now: u64,
    ) -> Result<Outcome> {
        let mut a = [0i16; 8];
        match i.arguments {
            Arguments::Bytes(v) => {
                a[0] = v[0].into();
                a[1] = v[1].into();
            }
            Arguments::Words(v) => a[..4].copy_from_slice(&v),
            Arguments::SlotString { slot, .. } => a[0] = slot.into(),
            _ => {}
        }
        if let Some(v) = i.continuation {
            a[4..].copy_from_slice(&v);
        }
        let text = match &i.arguments {
            Arguments::String(s) | Arguments::SlotString { text: s, .. } => s.as_str(),
            _ => "",
        };
        let mut next = i.next;
        let mut outcome = Outcome::Advanced;
        let event=match i.opcode {
            // Parser leaves PC on 0; native main only yields (LABEL_989).
            // An end marker is an idle menu point, not a process-exit opcode.
            0x00=>{self.state.wait_click=true;next=i.offset;outcome=Outcome::WaitForInput;None},
            0x04=>{let label=a[0] as u16 | (a[1] as u16)<<8;let Some(scene)=host.scene_index(&self.state.scene)?else{return Ok(Outcome::Pending)};if scene>=0&&label!=0{self.state.system_data.mark_read(scene as usize,label,label)?;self.state.read.insert((self.state.scene.clone(),label));}Some(Event::ReadLabel{scene:self.state.scene.clone(),label})},
            0x03=>{
                let label=a[0] as u16 | (a[1] as u16)<<8;
                let raw=*program.labels.get(usize::from(label)).context("Circus native read/save label index")?;
                let metadata=*program.label_metadata.get(usize::from(label)).context("Circus native label metadata")?;
                if program.instruction(next).map_or(true,|next|next.opcode!=0x4c) {
                    let extend=self.state.read_run.as_ref().is_some_and(|(scene,_,end)|scene==&self.state.scene&&u32::from(*end)+1==u32::from(label));
                    if extend {self.state.read_run.as_mut().unwrap().2=label;}
                    else {
                        if let Some((scene,start,end))=&self.state.read_run{let Some(id)=host.scene_index(scene)?else{return Ok(Outcome::Pending)};if id>=0&&*start!=0{self.state.system_data.mark_read(id as usize,*start,*end)?;}}
                        self.state.read_run=Some((self.state.scene.clone(),label,label));
                    }
                }
                self.state.savepoint=Some((self.state.scene.clone(),label,raw,metadata));None
            },
            0x06=>{self.state.wait_click=true;outcome=Outcome::WaitForInput;None},
            0x0b=>{self.state.finish_button();if !host.emit(&Event::BeginMenu(self.state.builder.clone()))?{return Ok(Outcome::Pending)}self.state.menu_wait=true;outcome=Outcome::WaitForInput;None},
            0x0a if a[0]!=0=>{if self.state.value(99,209)?!=0&&self.state.value(99,13)?!=0{self.state.assign(99,13,0)?;}None},
            0x0f if matches!(a[0],23|24|25|30|31|32)=>None, // Original dispatch explicitly excludes these modes from effects.
            // sub_407BE0 requires an existing transition colour resource.
            // All currently accepted portrait operations are immediate, so
            // sub_422680 has no animation queue to finish on this path.
            0x0f if a[0]<=1&&self.state.transition_color.is_none()=>None,
            0x0f if matches!(a[0],0|1|2)=>{
                ensure!(self.state.value(99,10)?!=0,"Circus disabled-effect cleanup policy");
                let out=a[0]==2;let color=if out{[0;3]}else{self.state.transition_color.context("Circus fade colour resource missing")?};
                let base=4*(5*i32::from(a[1])+5);let speed=self.state.value(99,12)?;
                let adjusted=if speed<5{base+((f64::from(base/2)/5.)*f64::from(5-speed)) as i32}else if speed>5{base-((f64::from(base/2)/6.)*f64::from(speed-5)) as i32}else{base};
                let duration_ms=fade_duration_ms(adjusted,!out)?;
                // Native425E60/407BE0 replace an active colour effect.
                self.state.transition_color=Some(color);self.state.transition_wait=Some(out);self.state.transition_poll_at=Some(now);
                Some(Event::BeginFade{out,duration_ms,color})
            },
            0x11=>{
                if a[0]==0 {self.state.bytes.retain(|key,_|*key>=1100);}
                if self.state.configured_names[0].is_empty(){self.state.configured_names[0]="常坂".into();}
                if self.state.configured_names[1].is_empty(){self.state.configured_names[1]="一登".into();}
                self.state.protagonist_names=self.state.configured_names.clone();
                self.state.system_data.set_names(&self.state.configured_names)?;
                self.state.savepoint=None;
                self.state.read_run=None;
                self.state.system.insert(233,0);
                for index in [26559,27964,27965,27966,27967,27968,28971,28970]{self.state.native_flags.insert(index,0);}
                None
            },
            0x15 if a[0]==254=>{self.state.wait_animations=true;outcome=Outcome::Pending;None},
            0x15=>{ensure!(a[0]<254,"unimplemented Circus timer mode {}",a[0]);if a[0]==0&&a[1]!=0{self.state.wait_click=true;outcome=Outcome::WaitForInput;}else{let until=now.saturating_add(a[0] as u64*200);self.state.wait_until=Some(until);self.state.wait_skippable=a[1]!=0;outcome=Outcome::WaitUntil(until);}None},
            0x16=>{let mut mask=a[0] as u8;while let Ok(data)=program.instruction(next){if data.opcode!=0x16{break;}let Arguments::Bytes([value,_])=data.arguments else{bail!("Circus audio wait marker");};mask|=value;next=data.next;}ensure!(mask==2,"Circus native audio wait mask {mask:#x}");self.state.wait_audio_mask=mask;outcome=Outcome::Pending;None},
            0x17=>{let b=self.state.current_button.as_mut().context("Circus button sounds without button")?;b.sounds=[a[0] as u8,a[1] as u8];None},
            0x18 if a[0]==0=>{let slot=u16::try_from(self.state.value(99,254)?)?;ensure!(!host.save_exists(slot)?,"Circus native original save loading not yet implemented");None},
            0x18 if a[0]==15=>{self.state.native_flags.insert(15632,a[1].into());self.state.native_flags.insert(11484,0);self.state.native_flags.insert(11488,0);self.state.assign(99,195,0)?;None}, // sub_421C90
            0x18 if a[0]==7=>{self.state.system.insert(3,i32::from(a[1]));None}, // native window member6; getter99/3.
            0x18 if a[0]==26=>{
                if self.state.pending_system_reload.is_none(){let Some(loaded)=host.reload_system()?else{return Ok(Outcome::Pending)};self.state.pending_system_reload=Some(loaded);}
                let loaded=self.state.pending_system_reload.as_ref().unwrap();
                let mut data=self.state.system_data.clone();
                let replay_music=matches!(loaded,SystemReload::Missing)&&self.state.value(99,14)?==0;
                match loaded{SystemReload::Loaded(loaded)=>data=loaded.clone(),SystemReload::Missing=>data.reset_preferences()}
                let names=data.names()?;
                let replay=if replay_music&&self.state.last_music.is_some(){
                    // A formerly nonlooping track must acquire real loop
                    // metadata before replaying; keep the completed reload
                    // instead of reissuing a conflicting storage worker.
                    let Some(asset)=host.music(u16::try_from(self.state.current_music)?,true)?else{return Ok(Outcome::Pending)};
                    ensure!(asset.name==self.state.last_music.as_ref().unwrap().name,"Circus reset retained music resource changed");Some(asset)
                }else{None};
                self.state.system_data=data;self.state.configured_names=names;
                // 40E9C0→43CE40(name,0,1) replays a retained music name after
                // restoring defaults; an empty filename produces no request.
                if let Some(asset)=replay{self.state.last_music=Some(asset.clone());self.state.pending_music=Some((asset,true));}
                self.state.pending_system_reload=None;
                None
            }, // 42B8B6 case1A →40EB40; this is a reload, not a no-op.
            0x18 if a[0]==5=>{
                // 403370 marks the current read label, then40C1C0 writes the
                // original two bounded preference blocks. Slot saves differ.
                if let Some((scene,start,end))=&self.state.read_run{let Some(index)=host.scene_index(scene)?else{return Ok(Outcome::Pending)};if index>=0&&*start!=0{self.state.system_data.mark_read(index as usize,*start,*end)?;}}
                if !host.write_system(self.state.system_data.bytes())?{return Ok(Outcome::Pending)}
                None
            },
            0x1e if a[0]==51&&a[1]==1=>{let unchanged=self.state.protagonist_names[0]=="常坂"&&self.state.protagonist_names[1]=="一登";self.state.native_flags.insert(66516,i32::from(!unchanged));None}, // sub_40A2E0 compares the two original four-byte names, not mail.
            0x1e if a[0]==19=>{let slot=u16::try_from(self.state.value(99,254)?)?;let Some(readable)=host.save_readable(slot)?else{return Ok(Outcome::Pending)};self.state.assign(9,87,i32::from(!readable))?;None}, // 40A2E0/40C620: exact356324-byte read probe, not save loading.
            0x1e if a[0]==51=>Some(Event::PlayerInput),
            0x51=>{self.state.animation_prefix=text.into();None},
            0x43=>{
                let offset=match a[0]{0=>{self.state.callbacks.insert(10456,String::new());10584},1=>{self.state.callbacks.insert(10520,String::new());10648},2=>10456,3=>10520,4=>10712,5=>10776,10=>10840,11=>10904,12=>11032,13=>11096,14=>11160,15=>11288,_=>bail!("Circus native callback slot {} needs verified activation/timer policy",a[0])};
                let(encoded,_,error)=encoding_rs::SHIFT_JIS.encode(text);ensure!(!error&&encoded.len()<64,"Circus native callback label extent");self.state.callbacks.insert(offset,text.into());None
            },
            0x1a if a[0]==0=>{self.state.builder=Group::default();self.state.current_button=None;None},
            0x1a|0x24=>{
                let slot=if i.opcode==0x1a {match a[0]{1=>Some(9),2=>Some(12),3=>Some(11),6=>Some(10),7=>Some(13),13..=17=>Some(a[0] as u8+4),_=>None}}
                    else {match a[0]{1..=7=>Some(a[0] as u8-1),13|14=>Some(a[0] as u8+4),15|16=>Some(a[0] as u8-8),254|255=>return self.emit_and_advance(i,host,Event::GroupVisibility(a[0]==254)),_=>None}};
                if let Some(slot)=slot {self.state.finish_button();let group=std::mem::take(&mut self.state.builder);self.state.groups.insert(slot,group.clone());Some(Event::DefineGroup{slot,group})}
                else {None} // Explicit native switch default is a no-op (0x42b3b7).
            },
            0x20=>{if a[0]==0 {next=skip(program,next,a[1] as u8)?;}None},
            0x32=>{self.state.builder.behavior_flags.insert(242);None}, // 412BE0 unconditionally sets group member242, regardless of operand.
            // Native dispatch calls CRT rand once, writes99/2 and records the
            // first200 results through402320 for original save/replay policy.
            // The shared sequence may be leased to the particle worker.
            0x25=>{ensure!(a[0]>0,"Circus native random modulo domain");let Some(value)=host.random(a[0] as u16)? else{return Ok(Outcome::Pending)};self.state.assign(99,2,value)?;if self.state.random_history.len()<200{self.state.random_history.push(value);}None},
            0x28=>Some(Event::FallingEffect{kind:a[0] as u8,parameter:a[1] as u8}),
            0x78=>{if self.state.builder.motion.is_none(){self.state.builder.position=[a[0],a[1]];if a[..2]!=a[2..4]{self.state.builder.motion=Some(GroupMotion{from:[a[0],a[1]],to:[a[2],a[3]],duration_ticks:20});}}None},
            0x34 if (1..=4).contains(&a[0])=>{self.state.builder.behavior_flags.insert(216+a[0] as u8);if a[0]==4{self.state.builder.behavior_flags.insert(244);}None},
            0x34 if a[0]==0=>{self.state.current_button.as_mut().context("Circus alpha-hit flag without current button")?.behavior_flags.insert(213);None},
            0x35=>{self.state.current_button.as_mut().context("Circus button flag without current button")?.behavior_flags.insert(214);None}, // sub_412870: builder[214] = 1.
            0x3d=>{ensure!((0..=5).contains(&a[0]),"Circus system image slot");let slot=a[0] as u8+37;let name=if text=="-"{self.state.image_names.get(&slot).context("Circus retained image undefined")?.clone()}else{text.into()};let event=Event::LoadImage{slot,name:name.clone()};if !host.emit(&event)?{return Ok(Outcome::Pending);}self.state.image_names.insert(slot,name);None},
            0x3c if a[0]==0=>None, // Exact native early-return for slot zero.
            0x3c=>{ensure!((1..=29).contains(&a[0]),"Circus staged image slot");ensure!(!text.starts_with('*')&&text!="-S","Circus indirect image resource policy");let slot=a[0] as u8;let event=Event::LoadImage{slot,name:text.into()};if !host.emit(&event)?{return Ok(Outcome::Pending);}self.state.image_names.insert(slot,text.into());self.state.pending_images.insert(slot);None},
            0x49=>{ensure!(!text.starts_with(':')&&text!="-S","Circus background display mode");ensure!(host.emit(&Event::ResetScene)?,"Circus scene reset must be synchronous");self.state.builder=Group::default();self.state.current_button=None;self.state.callbacks.clear();self.state.menu_wait=false;self.state.transition_wait=None;self.state.transition_poll_at=None;self.state.wait_animations=false;self.state.assign(99,246,0)?;for slot in &self.state.pending_images {ensure!(host.emit(&Event::DisplayImage{slot:*slot})?,"Circus publication must be synchronous");}self.state.pending_images.clear();None},
            0x4c|0x4d=>{self.state.pending_audio.retain(|(channel,_,_)|*channel!=8);self.state.pending_audio.push((8,text.into(),true));None},
            // Exact entries of native DC4PH jump table 0x431b10 targeting
            // common continuation 0x430d4a. Not an unknown-opcode fallback.
            0x01|0x02|0x08|0x0d|0x0e|0x12|0x14|0x19|0x1f|0x23|0x27|
            0x29..=0x31|0x3a|0x3b|0x42|0x48|0x4b|0x53|0x57|0x5d|
            0x60|0x62|0x66|0x69|0x6c|0x6e|0x8a|0x91|0x92|0x93|0x97|
            0x98|0xa3|0xa6=>None,
            0x54=>{next=program.instructions.iter().find(|candidate|candidate.opcode==0x4b&&matches!(&candidate.arguments,Arguments::String(label) if label==text)).context("Circus named jump label missing")?.offset;None},
            0x4f|0x50=>{
                let target=host.scene(text)?;let Some(target)=target else{return Ok(Outcome::Pending);};
                self.state.scene=text.into();self.program=Some(target);next=3;None
            },
            0x63=>{ensure!(self.state.version.len()+text.len()<=128,"Circus version string limit");self.state.version.push_str(text);None},
            0x65=>{if text.is_empty(){self.state.voice_prefixes.clear();}else if !self.state.voice_prefixes.iter().any(|s|s.eq_ignore_ascii_case(text)){ensure!(self.state.voice_prefixes.len()<50,"Circus voice prefix count");self.state.voice_prefixes.push(text.into());}None},
            0x68=>Some(Event::Cursor(text.into())),
            0x70 if a[0]==255=>{self.state.pending_music=None;self.state.last_music=None;self.state.current_music=255;Some(Event::StopAudio{channel:0})},
            0x70=>{ensure!((0..400).contains(&a[0]),"Circus music index");let looping=a[1]<2;let Some(asset)=host.music(a[0] as u16,looping)?else{return Ok(Outcome::Pending)};self.state.assign(96,a[0],1)?;self.state.last_music=Some(asset.clone());if !(a[1]==1&&self.state.current_music==a[0]){self.state.pending_music=Some((asset,looping));}self.state.current_music=a[0];None},
            0x71 if a[0]>=0=>{ensure!(a[1]==0,"Circus extended SE playback mode");let Some(name)=host.sound_name(a[0] as u16)?else{return Ok(Outcome::Pending);};let count=self.state.pending_audio.iter().filter(|(ch,_,_)|(2..=5).contains(ch)).count();if count<4{self.state.pending_audio.push((2+count as u32,name,false));}None},
            0x71 if a[0]==-2=>{self.state.pending_audio.retain(|(ch,_,_)|*ch<8);Some(Event::StopAudio{channel:8})},
            0x72=>{
                let data=program.instruction(next)?;let Arguments::String(action)=&data.arguments else{bail!("Circus button action must follow bounds as a string operand");};
                self.state.finish_button();self.state.current_button=Some(Button{bounds:a[..4].try_into()?,action:action.clone(),sounds:[0;2],behavior_flags:BTreeSet::new(),images:Vec::new(),texts:Vec::new()});next=data.next;None
            },
            0x73=>{
                ensure!(matches!(a[7],0|1|2|3|99),"Circus button image mode {}",a[7]);
                let b=self.state.current_button.as_mut().context("Circus state image without button")?;
                ensure!(b.images.len()<128,"Circus per-button image limit");b.images.push(image_part(a)?);None
            },
            0x7a=>{ensure!(self.state.builder.images.len()<4096,"Circus group image limit");let mut part=image_part(a)?;part.position[0]+=i32::from(self.state.builder_offset[0]);part.position[1]+=i32::from(self.state.builder_offset[1]);self.state.builder.images.push(part);None},
            0x7b=>{ensure!(a[1]<=256,"Circus extended UI animation command");self.state.builder.image_animations.push(a[..4].try_into()?);None},
            0x76=>{let data=program.instruction(next)?;let Arguments::String(text)=&data.arguments else{bail!("Circus button text operand");};let b=self.state.current_button.as_mut().context("Circus text without button")?;b.texts.push(TextPart{position:[a[0],a[1]],text:text.clone(),color:a[2],size:24,state:a[3]});next=data.next;None},
            0x82=>{let old=if a[3]==0{0}else{self.state.value(a[0],a[1])?};let value=match a[3]{0=>i32::from(a[2]),1=>old.wrapping_add(a[2].into()),2=>old.wrapping_sub(a[2].into()),_=>bail!("Circus arithmetic assignment mode {}",a[3])};self.state.assign(a[0],a[1],value)?;None},
            0x83=>{let value=self.value(host,a[2],a[3])?;self.state.assign(a[0],a[1],value)?;None},
            0x85=>{let value=self.state.value(a[0],a[1])?.wrapping_add(self.value(host,a[2],a[3])?);self.state.assign(a[0],a[1],value)?;None},
            0x86=>{
                let lhs=self.value(host,a[0],a[1])?;
                let mut comparison=a[3] as u8;let rhs=if comparison&0x80!=0 {comparison&=0x7f;self.value(host,(a[2] as u16>>8) as i16,(a[2] as u16&255) as i16)?}else{a[2].into()};
                let result=match comparison{0=>lhs==rhs,1=>lhs!=rhs,2=>lhs>rhs,3=>lhs<rhs,4=>lhs>=rhs,5=>lhs<=rhs,_=>bail!("Circus comparison {}",comparison)};
                let marker=program.instruction(next)?;ensure!(marker.opcode==2,"Circus conditional marker opcode");let Arguments::Bytes([id,_])=marker.arguments else{bail!("Circus conditional marker operands");};next=if result{marker.next}else{skip(program,marker.next,id)?};None
            },
            0x87 if a[2]==0=>None, // Original dispatch only enters its body for args[2] != 0.
            0x87=>{let data=program.instruction(next)?;let Arguments::String(text)=&data.arguments else{bail!("Circus group text operand");};self.state.builder.texts.push(TextPart{position:[a[0],a[1]],text:text.clone(),color:a[3],size:a[2],state:99});next=data.next;None},
            0x88=>{self.state.builder_offset=[a[0],a[1]];None}, // sub_412910
            // Native object 10/20 maps to resource 4/5 (sub_41E6A0).
            0x8b if a[3]==0 && (0..=1).contains(&a[2])=>Some(Event::SpritePosition{slot:a[2] as u8+4,position:[a[0],a[1]]}),
            // 0x42d071 explicitly selects [this+0x60cc] => native object75,
            // which sub_41E6A0 maps from resource31 (message-window object).
            0x8d=>Some(Event::SpritePosition{slot:31,position:[a[0],a[1]]}),
            0x90=>{ensure!((0..64).contains(&a[0]),"Circus palette index");Some(Event::Palette{index:a[0] as u8,rgb:[a[1] as u8,a[2] as u8,a[3] as u8]})},
            0xab if a[0]==99=>{ensure!(matches!(a[1],1|2),"Circus video backend ABI");self.state.video_backend=Some(a[1]);None}, // sub_457D90 selects a player; does not open/play media.
            _=>bail!("unsupported native Circus instruction {:#04x}; execution stopped without advancing",i.opcode),
        };
        if let Some(event) = event {
            if !host.emit(&event)? {
                return Ok(Outcome::Pending);
            }
        }
        self.state.pc = next;
        self.state.instructions += 1;
        Ok(outcome)
    }
    fn value(&self, host: &mut dyn NativeHost, bank: i16, index: i16) -> Result<i32> {
        if bank == 99 && index == 21 {
            return Ok(host.save_exists(2)? as i32);
        }
        self.state.value(bank, index)
    }
    fn emit_and_advance(
        &mut self,
        i: &Instruction,
        host: &mut dyn NativeHost,
        event: Event,
    ) -> Result<Outcome> {
        if !host.emit(&event)? {
            return Ok(Outcome::Pending);
        }
        self.state.pc = i.next;
        self.state.instructions += 1;
        Ok(Outcome::Advanced)
    }
}
fn image_part(a: [i16; 8]) -> Result<ImagePart> {
    ensure!(a[4] >= 0 && a[5] >= 0, "Circus image extent");
    ensure!((0..64).contains(&(a[6] & 0x3f)), "Circus bitmap slot");
    Ok(ImagePart {
        position: [i32::from(a[0]), i32::from(a[1])],
        source: [a[2], a[3], a[4], a[5]],
        slot: (a[6] & 0x3f) as u8,
        state: a[7],
        auxiliary_flag: (a[6] as i8) < 0,
    })
}
fn byte_bank(bank: i16, index: i16) -> Option<u16> {
    let (base, max) = match bank {
        100 => (2048, 149),
        96 => (2216, 399),
        97 => (2616, 1199),
        98 => (3816, 649),
        _ => return None,
    };
    if (0..=max).contains(&index) {
        Some(base)
    } else {
        None
    }
}
// Original 41BE90 selects a fixed 60 Hz timeline (4427C0(1000,60));
// 408A70 stores alpha as f32 at each tick. Retain its completion threshold.
fn fade_duration_ms(duration: i32, fade_in: bool) -> Result<u64> {
    ensure!((1..=20_000).contains(&duration), "Circus fade duration");
    let step = if fade_in {
        -0.5f32 / duration as f32
    } else {
        1f32 / duration as f32
    };
    let mut alpha = if fade_in { 1f32 } else { 0f32 };
    for frames in 1..=40_002u64 {
        alpha += step;
        if (fade_in && alpha <= 0.) || (!fade_in && alpha >= 1.) {
            return Ok((frames * 1000).div_ceil(60));
        }
    }
    bail!("Circus fade threshold limit")
}
fn skip(program: &Program, mut pc: u32, marker: u8) -> Result<u32> {
    for _ in 0..crate::MAX_INSTRUCTIONS {
        let i = program.instruction(pc)?;
        if i.opcode == 0 {
            bail!("Circus branch marker {marker} not found before end");
        }
        if i.opcode == 0x20 && matches!(i.arguments,Arguments::Bytes([_,id]) if id==marker) {
            return Ok(i.next);
        }
        pc = i.next;
    }
    bail!("Circus branch scan limit")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Host {
        program: Option<Arc<Program>>,
        events: Vec<Event>,
        pending: bool,
        playing: bool,
        system_pending: bool,
        system_writes: Vec<Vec<u8>>,
        save_is_readable: bool,
        reload_data: Option<crate::settings::SystemData>,
        reload_calls: usize,
        music_pending: bool,
        menu_action:Option<String>,
    }
    impl NativeHost for Host {
        fn menu_action(&mut self)->Result<Option<String>>{Ok(self.menu_action.take())}

        fn reload_system(&mut self) -> Result<Option<SystemReload>> {
            self.reload_calls += 1;
            if self.pending {
                self.pending = false;
                return Ok(None);
            }
            Ok(Some(
                self.reload_data
                    .clone()
                    .map_or(SystemReload::Missing, SystemReload::Loaded),
            ))
        }
        fn save_readable(&mut self, _slot: u16) -> Result<Option<bool>> {
            if self.pending {
                self.pending = false;
                return Ok(None);
            }
            Ok(Some(self.save_is_readable))
        }
        fn random(&mut self, modulus: u16) -> Result<Option<i32>> {
            if self.pending {
                self.pending = false;
                return Ok(None);
            }
            Ok(Some(7 % i32::from(modulus)))
        }
        fn scene_index(&mut self, _: &str) -> Result<Option<i32>> {
            Ok(Some(0))
        }
        fn write_system(&mut self, bytes: &[u8]) -> Result<bool> {
            if self.system_pending {
                self.system_pending = false;
                return Ok(false);
            }
            self.system_writes.push(bytes.to_vec());
            Ok(true)
        }
        fn music(&mut self, _: u16, looping: bool) -> Result<Option<MusicAsset>> {
            if self.music_pending {
                self.music_pending = false;
                return Ok(None);
            }
            Ok(Some(MusicAsset {
                name: "music\\authored.PCM".into(),
                loop_start: if looping { 100 } else { 0 },
                loop_end: if looping { Some(200) } else { None },
            }))
        }
        fn scene(&mut self, _: &str) -> Result<Option<Arc<Program>>> {
            Ok(self.program.clone())
        }
        fn emit(&mut self, e: &Event) -> Result<bool> {
            if self.pending {
                self.pending = false;
                return Ok(false);
            }
            self.events.push(e.clone());
            Ok(true)
        }
        fn save_exists(&mut self, _: u16) -> Result<bool> {
            Ok(false)
        }
        fn audio_playing(&mut self, _: u32) -> Result<bool> {
            Ok(self.playing)
        }
    }
    fn fixture(ops: Vec<(u8, Arguments)>) -> (Machine, Host) {
        let mut pc = 3;
        let instructions = ops
            .into_iter()
            .map(|(opcode, arguments)| {
                let i = Instruction {
                    offset: pc,
                    next: pc + 10,
                    opcode,
                    arguments,
                    continuation: None,
                };
                pc += 10;
                i
            })
            .collect();
        let program = Arc::new(Program {
            profile: Profile::Dc4PlusHarmony,
            code_file_offset: 0,
            labels: vec![0; 8],
            label_metadata: vec![0; 8],
            instructions,
        });
        (
            Machine::new("fixture"),
            Host {
                program: Some(program),
                ..Default::default()
            },
        )
    }
    fn b(a: u8, c: u8) -> Arguments {
        Arguments::Bytes([a, c])
    }
    fn w(a: i16, b: i16, c: i16, d: i16) -> Arguments {
        Arguments::Words([a, b, c, d])
    }
    #[test]
    fn conditional_consumes_marker_and_skips_native_else() {
        let (mut m, mut h) = fixture(vec![
            (0x86, w(0, 0, 1, 0)),
            (2, b(7, 0)),
            (0x82, w(0, 2, 11, 0)),
            (0x20, b(0, 7)),
            (0x82, w(0, 2, 22, 0)),
            (0x20, b(1, 7)),
            (0, b(0, 0)),
        ]);
        assert_eq!(m.step(&mut h, 0, false).unwrap(), Outcome::Advanced);
        assert_eq!(m.state.pc, 43);
        m.step(&mut h, 0, false).unwrap();
        assert_eq!(m.state.value(0, 2).unwrap(), 22);
        m.step(&mut h, 0, false).unwrap();
        assert_eq!(m.step(&mut h, 0, false).unwrap(), Outcome::WaitForInput);
    }
    #[test]
    fn byte_arithmetic_and_indirect_rhs() {
        let (mut m, mut h) = fixture(vec![
            (0x82, w(0, 1, 255, 0)),
            (0x82, w(0, 1, 2, 1)),
            (0x82, w(0, 2, 1, 0)),
            (0x86, w(0, 1, 2, 128)),
            (2, b(3, 0)),
            (0x82, w(0, 3, 9, 0)),
            (0x20, b(1, 3)),
            (0, b(0, 0)),
        ]);
        for _ in 0..5 {
            m.step(&mut h, 0, false).unwrap();
        }
        assert_eq!(m.state.value(0, 1).unwrap(), 1);
        assert_eq!(m.state.value(0, 3).unwrap(), 9);
    }
    #[test]
    fn wait_is_native_two_hundred_ms_and_input_is_separate() {
        let (mut m, mut h) = fixture(vec![(0x15, b(2, 0)), (6, b(0, 0)), (0, b(0, 0))]);
        assert_eq!(m.step(&mut h, 10, false).unwrap(), Outcome::WaitUntil(410));
        let pc = m.state.pc;
        assert_eq!(m.step(&mut h, 409, false).unwrap(), Outcome::WaitUntil(410));
        assert_eq!(m.state.pc, pc);
        assert_eq!(m.step(&mut h, 410, false).unwrap(), Outcome::WaitForInput);
        assert_eq!(m.step(&mut h, 999, false).unwrap(), Outcome::WaitForInput);
        assert_eq!(m.step(&mut h, 999, true).unwrap(), Outcome::WaitForInput);
    }
    #[test]
    fn button_action_is_data_not_an_executed_jump() {
        let (mut m, mut h) = fixture(vec![
            (0x72, w(10, 20, 50, 60)),
            (0x54, Arguments::String("nowhere".into())),
            (0x17, b(201, 200)),
            (0x1a, b(1, 0)),
            (0, b(0, 0)),
        ]);
        m.step(&mut h, 0, false).unwrap();
        assert_eq!(m.state.pc, 23);
        m.step(&mut h, 0, false).unwrap();
        m.step(&mut h, 0, false).unwrap();
        let group = m.state.groups.get(&9).unwrap();
        assert_eq!(group.buttons.len(), 1);
        assert_eq!(group.buttons[0].action, "nowhere");
        assert_eq!(group.buttons[0].sounds, [201, 200]);
    }
    #[test]
    fn pending_resource_keeps_pc_and_unknown_is_visible() {
        let (mut m, mut h) = fixture(vec![
            (
                0x3d,
                Arguments::SlotString {
                    slot: 0,
                    text: "SYSTEM\\MESWIN".into(),
                },
            ),
            (0xfe, w(0, 0, 0, 0)),
        ]);
        h.pending = true;
        assert_eq!(m.step(&mut h, 0, false).unwrap(), Outcome::Pending);
        assert_eq!(m.state.pc, 3);
        assert!(m.state.image_names.is_empty());
        m.step(&mut h, 0, false).unwrap();
        assert_eq!(m.state.image_names.get(&37).unwrap(), "SYSTEM\\MESWIN");
        let before = m.state.pc;
        let error = m.step(&mut h, 0, false).unwrap_err();
        assert!(format!("{error:#}").contains("opcode=0xfe"));
        assert_eq!(m.state.pc, before);
    }
    #[test]
    fn random_instruction_waits_for_shared_stream_without_repeating_history() {
        let (mut m, mut h) = fixture(vec![(0x25, b(10, 0)), (0, b(0, 0))]);
        h.pending = true;
        assert_eq!(m.step(&mut h, 0, false).unwrap(), Outcome::Pending);
        assert_eq!(m.state.pc, 3);
        assert!(m.state.random_history.is_empty());
        assert_eq!(m.step(&mut h, 0, false).unwrap(), Outcome::Advanced);
        assert_eq!(m.state.value(99, 2).unwrap(), 7);
        assert_eq!(m.state.random_history, [7]);
        assert_eq!(m.state.pc, 13);
    }
    #[test]
    fn native_group_slide_is_a_declaration_and_activation_stays_explicit() {
        let (mut m, mut h) = fixture(vec![
            (0x78, w(1280, 0, 0, 0)),
            (0x32, b(1, 0)),
            (0x0b, b(0, 0)),
        ]);
        assert_eq!(m.step(&mut h, 0, false).unwrap(), Outcome::Advanced);
        assert_eq!(m.state.builder.position, [1280, 0]);
        let motion = m.state.builder.motion.as_ref().unwrap();
        assert_eq!(motion.to, [0, 0]);
        assert_eq!(motion.duration_ticks, 20);
        assert!(h.events.is_empty());
        assert_eq!(m.step(&mut h, 0, false).unwrap(), Outcome::Advanced);
        assert!(m.state.builder.behavior_flags.contains(&242));
        assert_eq!(m.step(&mut h,0,false).unwrap(),Outcome::WaitForInput);
        assert!(matches!(h.events.last(),Some(Event::BeginMenu(_))));
        let before=m.state.pc;
        assert_eq!(m.step(&mut h,1000,true).unwrap(),Outcome::WaitForInput);
        assert_eq!(m.state.pc,before,"raw click is not an engine widget action");
    }
    #[test]
    fn native_background49_resets_effect_and_current_groups_before_publication(){
        let(mut m,mut h)=fixture(vec![(0x3c,Arguments::SlotString{slot:1,text:"bg".into()}),(0x49,Arguments::String("bg".into()))]);
        m.step(&mut h,0,false).unwrap();h.events.clear();m.state.callbacks.insert(10520,"old_action".into());m.state.builder.position=[100,0];
        m.step(&mut h,1,false).unwrap();assert!(matches!(h.events.as_slice(),[Event::ResetScene,Event::DisplayImage{slot:1}]));assert!(m.state.callbacks.is_empty());assert!(m.state.builder.buttons.is_empty());assert_eq!(m.state.builder.position,[0,0]);
    }
    #[test]
    fn process11_only_resumes_at_real_label_after_engine_activation(){
        let(mut m,mut h)=fixture(vec![(0x0b,b(0,0)),(0x82,w(0,0,99,0)),(0x4b,Arguments::String("chosen".into())),(0x82,w(0,0,7,0)),(0,b(0,0))]);
        h.pending=true;assert_eq!(m.step(&mut h,0,false).unwrap(),Outcome::Pending);assert_eq!(m.state.pc,3);
        assert_eq!(m.step(&mut h,1,false).unwrap(),Outcome::WaitForInput);
        for t in [10,20,30]{assert_eq!(m.step(&mut h,t,true).unwrap(),Outcome::WaitForInput);assert_eq!(m.state.value(0,0).unwrap(),0);}
        h.menu_action=Some("chosen".into());assert_eq!(m.step(&mut h,40,false).unwrap(),Outcome::Advanced);assert_eq!(m.state.pc,33);
        m.step(&mut h,40,false).unwrap();assert_eq!(m.state.value(0,0).unwrap(),7);
        let(mut m,mut h)=fixture(vec![(0x0b,b(0,0))]);m.step(&mut h,0,false).unwrap();h.menu_action=Some("missing".into());let before=m.state.pc;assert!(m.step(&mut h,1,false).is_err());assert_eq!(m.state.pc,before);
    }
    #[test]
    fn original_save_availability_probe_waits_and_sets_the_native_disable_byte() {
        let (mut m, mut h) = fixture(vec![(0x1e, b(19, 0)), (0x1e, b(19, 0))]);
        m.state.assign(99, 254, 1).unwrap();
        h.pending = true;
        assert_eq!(m.step(&mut h, 0, false).unwrap(), Outcome::Pending);
        assert_eq!(m.state.pc, 3);
        assert_eq!(m.step(&mut h, 0, false).unwrap(), Outcome::Advanced);
        assert_eq!(m.state.value(9, 87).unwrap(), 1);
        h.save_is_readable = true;
        assert_eq!(m.step(&mut h, 0, false).unwrap(), Outcome::Advanced);
        assert_eq!(m.state.value(9, 87).unwrap(), 0);
    }
    #[test]
    fn system_reload26_waits_then_replaces_preferences_without_resetting_script_state() {
        let (mut m, mut h) = fixture(vec![(0x18, b(26, 0))]);
        m.state.assign(2, 3, 9).unwrap();
        m.state.system_data.assign(7, 3).unwrap();
        let mut loaded = crate::settings::SystemData::default();
        loaded.assign(7, 81).unwrap();
        loaded.set_names(&["明日香".into(), "一登".into()]).unwrap();
        loaded.set_byte(1200, 0xa5);
        loaded.set_byte(crate::settings::SETTINGS_BYTES + 100, 0x5a);
        h.reload_data = Some(loaded.clone());
        h.pending = true;
        assert_eq!(m.step(&mut h, 0, false).unwrap(), Outcome::Pending);
        assert_eq!(m.state.pc, 3);
        assert_eq!(m.state.value(99, 7).unwrap(), 3);
        assert_eq!(m.step(&mut h, 0, false).unwrap(), Outcome::Advanced);
        assert_eq!(m.state.pc, 13);
        assert_eq!(m.state.instructions, 1);
        assert_eq!(m.state.system_data.bytes(), loaded.bytes());
        assert_eq!(m.state.configured_names, ["明日香", "一登"]);
        assert!(m.state.protagonist_names.iter().all(String::is_empty));
        assert_eq!(m.state.value(2, 3).unwrap(), 9);
        assert!(h.events.is_empty() && h.system_writes.is_empty());
    }
    #[test]
    fn missing_system_reload26_resets_only_the_native_preference_block() {
        let (mut m, mut h) = fixture(vec![(0x18, b(26, 0))]);
        m.state.system_data.assign(14, 0).unwrap();
        m.state.system_data.set_byte(1200, 0xa5);
        m.state
            .system_data
            .set_byte(crate::settings::SETTINGS_BYTES + 100, 0x5a);
        m.state.assign(2, 3, 9).unwrap();
        assert_eq!(m.step(&mut h, 0, false).unwrap(), Outcome::Advanced);
        let defaults = crate::settings::SystemData::default();
        assert_eq!(
            &m.state.system_data.bytes()[..crate::settings::SETTINGS_BYTES],
            &defaults.bytes()[..crate::settings::SETTINGS_BYTES]
        );
        assert_eq!(
            m.state
                .system_data
                .byte(crate::settings::SETTINGS_BYTES + 100),
            0x5a
        );
        assert_eq!(m.state.value(2, 3).unwrap(), 9);
        assert!(h.events.is_empty());
        assert!(m.state.pending_music.is_none());
    }
    #[test]
    fn missing_system_reset_reenables_and_replays_a_retained_music_name() {
        let (mut m, mut h) = fixture(vec![
            (0x70, w(1, 2, 0, 0)),
            (0x18, b(26, 0)),
            (0x15, b(1, 0)),
        ]);
        m.step(&mut h, 0, false).unwrap();
        assert!(m.state.last_music.as_ref().unwrap().loop_end.is_none());
        // Model a disabled category after its original resource name was set.
        // Reset restores the category and requests looping playback, not STOP.
        m.state.system_data.assign(14, 0).unwrap();
        m.state.pending_music = None;
        h.music_pending = true;
        assert_eq!(m.step(&mut h, 0, false).unwrap(), Outcome::Pending);
        assert_eq!(m.state.pc, 13);
        assert_eq!(m.state.value(99, 14).unwrap(), 0);
        assert_eq!(h.reload_calls, 1);
        m.step(&mut h, 0, false).unwrap();
        assert_eq!(
            h.reload_calls, 1,
            "completed storage reload must not be reissued while music metadata is pending"
        );
        assert_eq!(m.state.value(99, 14).unwrap(), 1);
        assert!(
            matches!(&m.state.pending_music,Some((asset,true))if asset.name=="music\\authored.PCM"&&asset.loop_start==100&&asset.loop_end==Some(200))
        );
        assert!(h.events.is_empty());
        assert_eq!(m.step(&mut h, 0, false).unwrap(), Outcome::WaitUntil(200));
        assert_eq!(m.step(&mut h, 1, false).unwrap(), Outcome::WaitUntil(200));
        assert!(matches!(
            h.events.as_slice(),
            [Event::PlayMusic { looping: true, .. }]
        ));
    }
    #[test]
    fn native_fade_without_resource_is_noop_but_resource_path_is_not_faked() {
        let (mut machine, mut host) = fixture(vec![(0x0f, b(0, 3)), (0x0f, b(0, 3))]);
        machine.step(&mut host, 0, false).unwrap();
        assert_eq!(machine.state.pc, 13);
        machine.state.transition_color = Some([0; 3]);
        machine.step(&mut host, 0, false).unwrap();
        assert!(matches!(host.events.last(), Some(Event::BeginFade { .. })));
        assert!(machine.step(&mut host, 1, false).is_err());
    }
    #[test]
    fn voice_wait_uses_engine_completion_without_advancing_pc() {
        let (mut machine, mut host) =
            fixture(vec![(0x16, b(2, 0)), (0x82, w(0, 1, 7, 0)), (0, b(0, 0))]);
        host.playing = true;
        assert_eq!(machine.step(&mut host, 0, false).unwrap(), Outcome::Pending);
        assert_eq!(machine.state.pc, 13);
        assert_eq!(
            machine.step(&mut host, 16, false).unwrap(),
            Outcome::Pending
        );
        assert_eq!(machine.state.pc, 13);
        host.playing = false;
        assert_eq!(
            machine.step(&mut host, 32, false).unwrap(),
            Outcome::Advanced
        );
        assert_eq!(machine.state.value(0, 1).unwrap(), 7);
    }
    #[test]
    fn native_player_input_wait_and_protagonist_name_test_are_distinct() {
        let (mut machine, mut host) =
            fixture(vec![(0x1e, b(51, 0)), (0x1e, b(51, 1)), (0x1e, b(51, 1))]);
        host.pending = true;
        assert_eq!(machine.step(&mut host, 0, false).unwrap(), Outcome::Pending);
        assert_eq!(machine.state.pc, 3);
        assert_eq!(
            machine.step(&mut host, 16, false).unwrap(),
            Outcome::Advanced
        );
        assert!(matches!(host.events.last(), Some(Event::PlayerInput)));
        machine.step(&mut host, 32, false).unwrap();
        assert_eq!(machine.state.native_flags.get(&66516), Some(&1));
        machine.state.protagonist_names = ["常坂".into(), "一登".into()];
        machine.step(&mut host, 48, false).unwrap();
        assert_eq!(machine.state.native_flags.get(&66516), Some(&0));
    }
    #[test]
    fn native_preference_write_is_an_acknowledged_barrier_and_value49_aliases_speed() {
        let (mut m, mut h) = fixture(vec![
            (0x82, w(100, 49, 37, 0)),
            (0x82, w(0, 1, 3, 0)),
            (0, b(0, 0)),
        ]);
        m.step(&mut h, 0, false).unwrap();
        h.system_pending = true;
        assert_eq!(m.state.value(99, 11).unwrap(), 37);
        assert_eq!(m.state.value(100, 49).unwrap(), 37);
        assert_eq!(m.step(&mut h, 16, false).unwrap(), Outcome::Pending);
        assert_eq!(m.state.pc, 13);
        assert_eq!(m.state.value(0, 1).unwrap(), 0);
        m.step(&mut h, 32, false).unwrap();
        assert_eq!(m.state.value(0, 1).unwrap(), 3);
        assert_eq!(h.system_writes.len(), 1);
        assert_eq!(h.system_writes[0].len(), crate::settings::SYSTEM_BYTES);
        assert_eq!(h.system_writes[0][265], 37);
    }
    #[test]
    fn music_repeat_keeps_native_same_id_suppression_and_engine_loop_points() {
        let (mut m, mut h) = fixture(vec![
            (0x70, w(1, 1, 0, 0)),
            (0x70, w(1, 1, 0, 0)),
            (0x15, b(254, 0)),
            (0, b(0, 0)),
        ]);
        for _ in 0..4 {
            m.step(&mut h, 0, false).unwrap();
        }
        assert_eq!(
            h.events
                .iter()
                .filter(|e| matches!(e, Event::PlayMusic { .. }))
                .count(),
            1
        );
        let Some(Event::PlayMusic { asset, looping }) = h.events.first() else {
            panic!("music must be flushed before native yield")
        };
        assert!(*looping);
        assert_eq!((asset.loop_start, asset.loop_end), (100, Some(200)));
        assert_eq!(m.state.value(96, 1).unwrap(), 1);
    }
    #[test]
    fn mixer_defaults_preserve_native_centibel_curve_and_bus_enable_policy() {
        let mut state = State::new("authored".into());
        let controls = state.audio_controls().unwrap();
        assert_eq!(controls.enabled, [true; 3]);
        assert!((controls.gain[0] - 0.1).abs() < 1e-6);
        assert!((controls.gain[1] - 0.17782794).abs() < 1e-6);
        assert_eq!(controls.gain[2], 1.);
        state.assign(99, 225, 50).unwrap();
        assert!((state.audio_controls().unwrap().gain[0] - 0.017782794).abs() < 1e-6);
        state.assign(99, 15, 0).unwrap();
        assert!(!state.audio_controls().unwrap().enabled[1]);
    }
}
