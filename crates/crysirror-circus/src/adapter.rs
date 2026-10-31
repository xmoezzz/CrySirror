//! Native MES resource/wait policy mapped to client-scoped CrySirror services.
//! Reading and decoding are workers; only the VM owner publishes engine state.
use crate::{
    machine::{Event, Group, ImagePart, Machine, NativeHost, Outcome},
    Program,
};
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{
        AudioParameters, AudioPreparePhase, AudioRequest, ClientStorage, EncodedAudio, EngineQuery,
        HostServices, InputFrame, PcmBus, UploadedImage,
    },
    state::ClientId,
    vm::{VmAdapter, VmYield},
};
use crysirror_image::{cursor::CursorImage, ImageSurface};
use crysirror_resource::{Archive, ArchiveFormat, DecodeContext};
#[cfg(feature = "runtime")]
use crysirror_runtime::{model::ClientSpec, runtime::VmFactory, AdapterRegistry};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
};
mod audio;
mod effects;
mod profile;
mod menu;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Native scene basename, relative to Advdata/MES (no generated bootstrap).
    pub script: String,
    /// Licensed project font alias; no fonts or user game files are embedded.
    pub font: String,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        scene_path(&self.script)?;
        ensure!(
            !self.font.is_empty()
                && self.font.len() <= 256
                && !self.font.chars().any(char::is_control),
            "Circus font alias"
        );
        Ok(())
    }
}
#[cfg(feature = "runtime")]
struct Factory;
#[cfg(feature = "runtime")]
impl VmFactory for Factory {
    fn create(&self, client: ClientId, spec: &ClientSpec) -> Result<Box<dyn VmAdapter>> {
        ensure!(
            spec.program.is_empty(),
            "Circus executes native MES, not data-VM actions"
        );
        Ok(Box::new(CircusVm::new(
            client,
            serde_json::from_value(spec.config.clone())?,
        )?))
    }
}
#[cfg(feature = "runtime")]
pub fn register(registry: &mut AdapterRegistry) -> Result<()> {
    registry.register("circus", Arc::new(Factory))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Work {
    Scene(String),
    Image(String),
    Cursor(String),
    Names,
    Audio(String, bool),
    ProfileLoad,
    ProfileSave(profile::ProfileData),
    SystemLoad,
    SystemReload,
    SystemSave(Vec<u8>),
    MusicLoops,
    SaveProbe(u16),
}
enum Prepared {
    Scene(Arc<Program>),
    Image(ImageSurface, [u32; 2]),
    Cursor(CursorImage),
    Names(crate::names::Names),
    Audio(Arc<[u8]>, Option<Arc<dyn Archive>>),
    Profile(profile::ProfileData),
    ProfileSaved,
    System(crate::settings::SystemData),
    SystemReload(crate::machine::SystemReload),
    SystemSaved,
    MusicLoops(BTreeMap<String, [u64; 2]>),
    SaveReadable(bool),
}
struct Loading {
    work: Work,
    receiver: Receiver<std::result::Result<Prepared, String>>,
}
#[derive(Default)]
struct Services {
    storage: Option<Arc<dyn ClientStorage>>,
    system_loaded: bool,
    loading: Option<Loading>,
    scenes: BTreeMap<String, Arc<Program>>,
    images: BTreeMap<u8, UploadedImage>,
    image_extents: BTreeMap<u8, [u32; 2]>,
    image_names: BTreeMap<u8, String>,
    ui_ids: BTreeMap<u8, Vec<u32>>,
    layer_ids: BTreeMap<u8, Vec<u32>>,
    palette: BTreeMap<u8, [u8; 3]>,
    positions: BTreeMap<u8, [i16; 2]>,
    sprite_scales: BTreeMap<u8,(i16,[i16;2])>,
    names: Option<crate::names::Names>,
    voice_archive: Option<Arc<dyn Archive>>,
    audio: Option<(u32, String, u64)>,
    music_loops: Option<BTreeMap<String, [u64; 2]>>,
    music_release_pending: Option<crysirror_core::service::AudioReleaseHandle>,
    music_releases: Vec<(crysirror_core::service::AudioReleaseHandle, u64)>,
    audio_update_due: bool,
    mixer_controls: Option<crate::machine::MixerControls>,
    scene_root: bool,
    displayed: BTreeSet<u8>,
    fit_background:bool,
    fade: Option<(bool, u32, u32)>,
    fade_canvas: bool,
    fade_frames: u64,
    fade_duration_ms: u64,
    frame_at: Option<u64>,
    profile: Option<profile::ProfileInput>,
    profile_captured: bool,
    sequence_rng: Option<crysirror_vfx::falling_atlas::SequenceRng>,
    effects: Option<effects::NativeEffect>,
    effect_starting: bool,
    effect_stopping: bool,
    random_exchange: Option<(u16, i32)>,
    menu:Option<menu::NativeMenu>,
    menu_images:BTreeMap<u8,Arc<dyn crysirror_core::image_processing::ImageResource>>,
    menu_image_job:Option<(u8,crysirror_core::image_processing::ImageJob)>,
}
const SCENE_ROOT: u32 = 0x3400_0000;
const COLOUR_ROOT: u32 = 0x3400_0001;
// Canvas presentation roots reserve the upper 0x2000_0000 namespace.
const FADE_SOURCE: u32 = 0x1400_0002;
const FADE_CANVAS: u32 = 0x3300_0000;
const CHECKPOINT_CANVAS:u32=0x3300_0002;
pub struct CircusVm {
    client: ClientId,
    config: Config,
    machine: Machine,
    services: Services,
}
impl CircusVm {
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        let machine = Machine::new(config.script.clone());
        Ok(Self {
            client,
            config,
            machine,
            services: Services::default(),
        })
    }
}
fn scene_path(name: &str) -> Result<VfsPath> {
    ensure!(
        !name.is_empty() && name.len() <= 128 && !name.contains(['\\', '/', ':']),
        "Circus scene basename"
    );
    Ok(VfsPath::new(
        Mount::Asset,
        &format!("Advdata/MES/{}.mes", name.trim_end_matches(".mes")),
    )?)
}
fn image_path(name: &str) -> Result<VfsPath> {
    let native = name.replace('\\', "/");
    Ok(VfsPath::new(
        Mount::Asset,
        &format!("Advdata/GRP/{native}.crx"),
    )?)
}
impl Services {
    fn prepare(&mut self, work: Work, client: ClientId) -> Result<Option<Prepared>> {
        if let Some(job) = self.loading.as_ref() {
            ensure!(
                job.work == work,
                "Circus worker completion ownership mismatch"
            );
            match job.receiver.try_recv() {
                Err(TryRecvError::Empty) => return Ok(None),
                Err(TryRecvError::Disconnected) => bail!("Circus resource worker disconnected"),
                Ok(value) => {
                    self.loading = None;
                    return value.map(Some).map_err(anyhow::Error::msg);
                }
            }
        }
        let storage = self
            .storage
            .as_ref()
            .context("Circus storage not initialized")?
            .clone();
        let archive = self.voice_archive.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        let task = work.clone();
        std::thread::Builder::new()
            .name("circus-native-resource".into())
            .spawn(move || {
                let result = (|| -> Result<Prepared> {
                    match task {
                        Work::SaveProbe(slot) => {
                            let path = VfsPath::new(Mount::Save, &format!("DC4PHDL{slot:02}.dat"))?;
                            let Some(metadata) = storage.metadata(Mount::Save, path.relative())?
                            else {
                                return Ok(Prepared::SaveReadable(false));
                            };
                            if metadata.is_directory {
                                return Ok(Prepared::SaveReadable(false));
                            }
                            let reader = storage.open_read(&path)?;
                            if reader.len()? < crate::native_save::BODY_BYTES as u64 {
                                return Ok(Prepared::SaveReadable(false));
                            }
                            let mut block = vec![0; crate::native_save::BODY_BYTES];
                            crysirror_resource::read_exact_at(reader.as_ref(), 0, &mut block)?;
                            Ok(Prepared::SaveReadable(true))
                        }
                        Work::Scene(name) => {
                            let data = storage.read(&scene_path(&name)?)?;
                            Ok(Prepared::Scene(Arc::new(Program::parse(&data)?)))
                        }
                        Work::Image(name) => {
                            let data = storage.read(&image_path(&name)?)?;
                            let decoded = crysirror_image::crx::decode(&data)?.surface;
                            let logical = [decoded.width, decoded.height];
                            // Native 44C130/44BFB0 + 44A590 allocate backing
                            // textures with independently rounded POT sides.
                            let extent = [
                                decoded.width.max(2).next_power_of_two(),
                                decoded.height.max(2).next_power_of_two(),
                            ];
                            let surface =
                                crysirror_image::padding::pad_to_extent(&decoded, extent, [0; 4])?;
                            Ok(Prepared::Image(surface, logical))
                        }
                        Work::Cursor(name) => {
                            let path =
                                VfsPath::new(Mount::Asset, &format!("Advdata/GRP/CURSOR/{name}"))?;
                            let data = storage.read(&path)?;
                            Ok(Prepared::Cursor(crysirror_image::cursor::decode_cursor(
                                &data,
                            )?))
                        }
                        Work::Names => {
                            let data = storage
                                .read(&VfsPath::new(Mount::Asset, "Advdata/GRP/NAMES.DAT")?)?;
                            Ok(Prepared::Names(crate::names::Names::parse(&data)?))
                        }
                        Work::ProfileLoad => {
                            let path = VfsPath::new(Mount::Save, "circus-profile.json")?;
                            let profile =
                                if storage.metadata(Mount::Save, path.relative())?.is_some() {
                                    let bytes = storage.read(&path)?;
                                    ensure!(bytes.len() <= 4096, "Circus local profile byte limit");
                                    serde_json::from_slice::<profile::ProfileData>(&bytes)?
                                } else {
                                    profile::ProfileData::default()
                                };
                            profile.validate()?;
                            Ok(Prepared::Profile(profile))
                        }
                        Work::ProfileSave(profile) => {
                            profile.validate()?;
                            storage.write_atomic(
                                &VfsPath::new(Mount::Save, "circus-profile.json")?,
                                &serde_json::to_vec(&profile)?,
                            )?;
                            Ok(Prepared::ProfileSaved)
                        }
                        Work::SystemLoad => {
                            let path = VfsPath::new(Mount::Save, "system.dat")?;
                            let settings =
                                if storage.metadata(Mount::Save, path.relative())?.is_some() {
                                    crate::settings::SystemData::try_from(storage.read(&path)?)?
                                } else {
                                    crate::settings::SystemData::default()
                                };
                            settings.names()?;
                            Ok(Prepared::System(settings))
                        }
                        Work::SystemReload => {
                            let path = VfsPath::new(Mount::Save, "system.dat")?;
                            let loaded =
                                if storage.metadata(Mount::Save, path.relative())?.is_some() {
                                    let data = crate::settings::SystemData::try_from(
                                        storage.read(&path)?,
                                    )?;
                                    data.names()?;
                                    crate::machine::SystemReload::Loaded(data)
                                } else {
                                    crate::machine::SystemReload::Missing
                                };
                            Ok(Prepared::SystemReload(loaded))
                        }
                        Work::SystemSave(bytes) => {
                            ensure!(
                                bytes.len() == crate::settings::SYSTEM_BYTES,
                                "Circus system preference byte limit"
                            );
                            storage
                                .write_atomic(&VfsPath::new(Mount::Save, "system.dat")?, &bytes)?;
                            Ok(Prepared::SystemSaved)
                        }
                        Work::MusicLoops => Ok(Prepared::MusicLoops(audio::load_music_loops(
                            storage.as_ref(),
                        )?)),
                        Work::Audio(name, true) => {
                            let native = name.replace('\\', "/");
                            ensure!(
                                native
                                    .get(..6)
                                    .is_some_and(|s| s.eq_ignore_ascii_case("voice/")),
                                "Circus voice resource prefix"
                            );
                            let path = format!(
                                "{}.PCM",
                                native[6..]
                                    .trim_end_matches(".PCM")
                                    .trim_end_matches(".pcm")
                            );
                            let context = DecodeContext::new(client);
                            let archive = if let Some(a) = archive {
                                a
                            } else {
                                let reader = storage
                                    .open_read(&VfsPath::new(Mount::Asset, "Advdata/VOICE.DAT")?)?;
                                Arc::from(
                                    crysirror_resource::circus_pck::CircusPck
                                        .open(reader, &context)?,
                                )
                            };
                            let id = archive
                                .resolve(&path)
                                .context("Circus voice archive resource missing")?;
                            let reader = archive.open(id, &context)?;
                            let bytes = crysirror_resource::read_all(
                                reader.as_ref(),
                                context.limits.max_entry_size,
                            )?;
                            Ok(Prepared::Audio(bytes.into(), Some(archive)))
                        }
                        Work::Audio(name, false) => {
                            let path = VfsPath::new(
                                Mount::Asset,
                                &format!("Advdata/{}", name.replace('\\', "/")),
                            )?;
                            let reader = storage.open_read(&path)?;
                            let bytes =
                                crysirror_resource::read_all(reader.as_ref(), 64 * 1024 * 1024)?;
                            Ok(Prepared::Audio(bytes.into(), None))
                        }
                    }
                })();
                let _ = sender.send(result.map_err(|e| format!("{e:#}")));
            })?;
        self.loading = Some(Loading { work, receiver });
        Ok(None)
    }
}
struct Bridge<'a> {
    services: &'a mut Services,
    host: &'a mut dyn HostServices,
    client: ClientId,
    font: &'a str,
    input:InputFrame,
}
impl Bridge<'_> {
    fn native_frame(&mut self) -> Result<()> {
        let now = self.host.now_ms();
        if self.services.frame_at == Some(now) {
            return Ok(());
        }
        self.services.frame_at = Some(now);
        self.services.audio_update_due = true;
        self.advance_effects(now)?;
        self.advance_menu()?;
        if let Some((_, target, _)) = self.services.fade {
            self.services.fade_frames += 1;
            let elapsed = (self.services.fade_frames * 1000)
                .div_ceil(60)
                .min(self.services.fade_duration_ms);
            self.submit(
                vec![json!({"op":"set_crossfade_elapsed","id":target,"elapsed_ms":elapsed})],
                vec![],
            )?;
        }
        Ok(())
    }
    /// Retained shared Layer owns native stage position/scale even when its
    /// image slot is currently empty; later images attach to the same parent.
    fn sprite_root(&mut self, slot:u8) -> Result<u32> {
        ensure!((4..=8).contains(&slot),"Circus native sprite slot");
        self.root()?;
        let id=0x3501_0000+u32::from(slot);
        let position=self.services.positions.get(&slot).copied().unwrap_or([0,0]);
        let (percent,pivot)=self.services.sprite_scales.get(&slot).copied().unwrap_or((100,[0,0]));
        //4432E0 uses percentages below1000 and hundredths of a percent
        //for both axes at/above1000. A zero pivot selects the source centre;
        //that case is resolved when a real image is attached below.
        let scale=f32::from(percent)/if percent>=1000{10000.}else{100.};
        let logical=self.services.image_extents.get(&slot).copied().unwrap_or([1280,720]);
        let pivot=[if pivot[0]==0{logical[0] as f32/2.}else{f32::from(pivot[0])},if pivot[1]==0{logical[1] as f32/2.}else{f32::from(pivot[1])}];
        self.submit(vec![json!({"op":"put","layer":{"id":id,"parent":SCENE_ROOT,"rect":[position[0],position[1],1280,720],"paint":{"kind":"solid","color":[0,0,0,0]},"order":[0,slot],"transform":{"scale_x":scale,"scale_y":scale,"pivot_x":pivot[0],"pivot_y":pivot[1]}}})],vec![])?;
        Ok(id)
    }
    fn root(&mut self) -> Result<()> {
        if !self.services.scene_root {
            self.submit(vec![json!({"op":"put","layer":{"id":SCENE_ROOT,"rect":[0,0,1280,720],"paint":{"kind":"solid","color":[0,0,0,255]},"order":[0,0]}})],vec![])?;
            self.services.scene_root = true;
        }
        Ok(())
    }
    fn submit(&mut self, layers: Vec<Value>, ui: Vec<Value>) -> Result<()> {
        self.host.submit_engine(
            self.client,
            1,
            &serde_json::to_vec(&json!({"layers":layers,"ui":ui}))?,
        )?;
        Ok(())
    }
    fn define_group(&mut self, slot: u8, group: &Group) -> Result<()> {
        let base = 0x3600_0000 + u32::from(slot) * 65536;
        let mut layers = self
            .services
            .layer_ids
            .get(&slot)
            .into_iter()
            .flatten()
            .map(|id| json!({"op":"remove","id":id}))
            .collect::<Vec<_>>();
        let mut ui = self
            .services
            .ui_ids
            .get(&slot)
            .into_iter()
            .flatten()
            .map(|id| json!({"op":"remove","id":id}))
            .collect::<Vec<_>>();
        let mut lids = Vec::new();
        let mut uids = Vec::new();
        for (n, part) in group.images.iter().enumerate() {
            let id = base + n as u32;
            self.image_layer(&mut layers, id, part, slot)?;
            lids.push(id);
        }
        for (n, b) in group.buttons.iter().enumerate() {
            let id = base + 0x8000 + n as u32;
            let [x, y, r, d] = b.bounds;
            ensure!(r >= x && d >= y, "Circus native button extent");
            ui.push(json!({"op":"put","widget":{"id":id,"rect":[x,y,r-x,d-y],"kind":"button","color":[0,0,0,0],"visible":false,"text":"","font":self.font}}));
            uids.push(id);
            for (k, part) in b.images.iter().enumerate() {
                let lid = base + 4096 + n as u32 * 128 + k as u32;
                self.image_layer(&mut layers, lid, part, slot)?;
                lids.push(lid);
            }
        }
        // Text declarations require exact native font state before publication.
        // Empty strings produce no engine glyphs, as in the original engine.
        ensure!(
            group
                .texts
                .iter()
                .chain(group.buttons.iter().flat_map(|b| &b.texts))
                .all(|t| t.text.is_empty()),
            "Circus nonempty native UI text needs verified font/markup policy"
        );
        self.submit(layers, ui)?;
        self.services.layer_ids.insert(slot, lids);
        self.services.ui_ids.insert(slot, uids);
        Ok(())
    }
    fn image_layer(&self, layers: &mut Vec<Value>, id: u32, p: &ImagePart, slot: u8) -> Result<()> {
        // Native null texture pointers are intentionally absent draws.
        let Some(image) = self.services.images.get(&p.slot) else {
            return Ok(());
        };
        let [sx, sy, w, h] = p.source;
        let [iw, ih] = image.size;
        if w == 0 || h == 0 {
            return Ok(());
        }
        ensure!(iw > 0 && ih > 0, "Circus image dimensions");
        layers.push(json!({"op":"put","layer":{"id":id,"rect":[p.position[0],p.position[1],w,h],"paint":{"kind":"image","asset":image.path.relative()},"sampling":{"address_u":"repeat","address_v":"repeat"},"visible":false,"order":[slot,id&65535],"uv":[sx as f32/iw as f32,sy as f32/ih as f32,(i32::from(sx)+i32::from(w)) as f32/iw as f32,(i32::from(sy)+i32::from(h)) as f32/ih as f32]}}));
        Ok(())
    }
}
impl NativeHost for Bridge<'_> {
    fn image_dimensions(&mut self,slot:u8)->Result<Option<([u32;2],[u32;2])>> {
        Ok(self.services.images.get(&slot).zip(self.services.image_extents.get(&slot)).map(|(image,logical)|(image.size,*logical)))
    }
    fn capture_checkpoint(&mut self)->Result<Option<Vec<u8>>> {
        if !self.effects_ready()? {return Ok(None)}
        ensure!(self.services.loading.is_none()&&self.services.audio.is_none(),"Circus checkpoint requires completed resource publication");
        self.root()?;
        self.host.submit_engine(self.client,1,&serde_json::to_vec(&json!({"canvases":[
            {"op":"create","id":CHECKPOINT_CANVAS,"width":1280,"height":720},
            {"op":"copy_subtree","source":SCENE_ROOT,"destination":CHECKPOINT_CANVAS,"rect":[0,0,1280,720],"position":[0,0]}
        ]}))?)?;
        let visual:Value=serde_json::from_slice(&self.host.query_engine(self.client,EngineQuery::Canvas(CHECKPOINT_CANVAS))?)?;
        ensure!(!visual.is_null(),"Circus checkpoint needs actual shared canvas receipt");
        let ui:Value=serde_json::from_slice(&self.host.query_engine(self.client,EngineQuery::UiSnapshot{canvas:[1280,720],root:None})?)?;
        let mut ids=BTreeSet::from([SCENE_ROOT,COLOUR_ROOT]);
        ids.extend(self.services.images.keys().map(|slot|0x3700_0000+u32::from(*slot)));
        ids.extend((4..=8).map(|slot|0x3501_0000+slot));
        ids.extend(self.services.layer_ids.values().flatten().copied());
        let mut retained=Vec::new();for id in ids {let layer:Value=serde_json::from_slice(&self.host.query_engine(self.client,EngineQuery::Layer(id))?)?;if !layer.is_null(){retained.push(layer)}}
        let resources=json!({"schema":1,"owner":self.client.0,"scene":visual,"ui":ui,"retained_layers":retained,"image_names":self.services.image_names,"positions":self.services.positions,"scales":self.services.sprite_scales,"palette":self.services.palette,"fit_background":self.services.fit_background,"sequence_rng":self.services.sequence_rng});
        Ok(Some(serde_json::to_vec(&resources)?))
    }

    fn menu_action(&mut self)->Result<Option<String>>{self.tick_menu()}

    fn reload_system(&mut self) -> Result<Option<crate::machine::SystemReload>> {
        let Some(Prepared::SystemReload(loaded)) =
            self.services.prepare(Work::SystemReload, self.client)?
        else {
            return Ok(None);
        };
        Ok(Some(loaded))
    }
    fn save_readable(&mut self, slot: u16) -> Result<Option<bool>> {
        let Some(Prepared::SaveReadable(readable)) =
            self.services.prepare(Work::SaveProbe(slot), self.client)?
        else {
            return Ok(None);
        };
        Ok(Some(readable))
    }
    fn random(&mut self, modulus: u16) -> Result<Option<i32>> {
        self.native_random(modulus)
    }
    fn music(&mut self, id: u16, looping: bool) -> Result<Option<crate::machine::MusicAsset>> {
        self.native_music(id, looping)
    }
    fn scene_index(&mut self, name: &str) -> Result<Option<i32>> {
        if self.services.names.is_none() {
            let Some(Prepared::Names(names)) = self.services.prepare(Work::Names, self.client)?
            else {
                return Ok(None);
            };
            self.services.names = Some(names);
        }
        Ok(Some(
            self.services
                .names
                .as_ref()
                .unwrap()
                .scenes
                .iter()
                .position(|candidate| candidate.eq_ignore_ascii_case(name))
                .map_or(-1, |id| id as i32),
        ))
    }
    fn write_system(&mut self, bytes: &[u8]) -> Result<bool> {
        let Some(prepared) = self
            .services
            .prepare(Work::SystemSave(bytes.to_vec()), self.client)?
        else {
            return Ok(false);
        };
        ensure!(
            matches!(prepared, Prepared::SystemSaved),
            "Circus system preference completion type"
        );
        Ok(true)
    }
    fn audio_playing(&mut self, channel: u32) -> Result<bool> {
        Ok(self.host.audio_playing(self.client, channel)?)
    }
    fn fade_complete(&mut self, out: bool) -> Result<bool> {
        let (expected, target, source) = self.services.fade.context("Circus fade owner missing")?;
        ensure!(
            expected == out,
            "Circus fade completion belongs to another operation"
        );
        let status: Value = serde_json::from_slice(
            &self
                .host
                .query_engine(self.client, EngineQuery::LayerCrossfade(target))?,
        )?;
        let elapsed = status
            .get("elapsed_ms")
            .and_then(Value::as_u64)
            .context("Circus engine fade status missing")?;
        let duration = status
            .get("duration_ms")
            .and_then(Value::as_u64)
            .context("Circus engine fade duration missing")?;
        if elapsed < duration {
            return Ok(false);
        }
        let mut commands = vec![
            json!({"op":"end_crossfade","id":target}),
            json!({"op":"set_visible","id":source,"visible":false}),
            json!({"op":"set_visible","id":target,"visible":true}),
        ];
        if out {
            commands.extend(
                self.services
                    .displayed
                    .iter()
                    .map(|slot| json!({"op":"remove","id":0x3700_0000+u32::from(*slot)})),
            );
        }
        self.submit(commands, vec![])?;
        if out {
            self.host.submit_engine(self.client,1,&serde_json::to_vec(&json!({"canvases":[{"op":"hide","layer":FADE_SOURCE},{"op":"clear","id":FADE_CANVAS,"color":[0,0,0,0]}]}))?)?;
            if self.services.profile_captured {
                profile::clear_capture(self.host, self.client)?;
                self.services.profile_captured = false;
            }
        }
        if out {
            self.services.displayed.clear();
            self.services
                .images
                .retain(|slot, _| (37..=44).contains(slot));
            self.services
                .image_names
                .retain(|slot, _| (37..=44).contains(slot));
            self.services
                .image_extents
                .retain(|slot, _| (37..=44).contains(slot));
        }
        self.services.fade = None;
        Ok(true)
    }
    fn scene(&mut self, name: &str) -> Result<Option<Arc<Program>>> {
        if let Some(p) = self.services.scenes.get(name) {
            return Ok(Some(p.clone()));
        }
        let Some(prepared) = self
            .services
            .prepare(Work::Scene(name.into()), self.client)?
        else {
            return Ok(None);
        };
        let Prepared::Scene(p) = prepared else {
            bail!("Circus scene worker type");
        };
        self.services.scenes.insert(name.into(), p.clone());
        Ok(Some(p))
    }
    fn save_exists(&mut self, slot: u16) -> Result<bool> {
        Ok(self
            .services
            .storage
            .as_ref()
            .context("Circus storage")?
            .metadata(Mount::Save, &format!("DC4PHDL{slot:02}.dat"))?
            .is_some())
    }
    fn sound_name(&mut self, id: u16) -> Result<Option<String>> {
        if self.services.names.is_none() {
            let Some(prepared) = self.services.prepare(Work::Names, self.client)? else {
                return Ok(None);
            };
            let Prepared::Names(names) = prepared else {
                bail!("Circus names worker type");
            };
            self.services.names = Some(names);
        }
        Ok(Some(
            self.services
                .names
                .as_ref()
                .unwrap()
                .sfx
                .get(usize::from(id))
                .context("Circus SFX index")?
                .clone(),
        ))
    }
    fn emit(&mut self, event: &Event) -> Result<bool> {
        match event {
            Event::PlayerInput => return self.player_input(),
            Event::BeginMenu(group)=>return self.begin_menu(group),
            Event::LoadImage { slot, name } => {
                if self.services.image_names.get(slot) == Some(name) {
                    return Ok(true);
                }
                let Some(prepared) = self
                    .services
                    .prepare(Work::Image(name.clone()), self.client)?
                else {
                    return Ok(false);
                };
                let Prepared::Image(image, logical) = prepared else {
                    bail!("Circus image worker type");
                };
                let path = self.host.upload_rgba(
                    self.client,
                    0x3500_0000 + u32::from(*slot),
                    image.width,
                    image.height,
                    &image.rgba,
                )?;
                let opaque = image.rgba.chunks_exact(4).all(|p| p[3] == 255);
                self.services.menu_images.remove(slot);
                self.services.images.insert(
                    *slot,
                    UploadedImage {
                        path,
                        size: [image.width, image.height],
                        opaque,
                    },
                );
                self.services.image_extents.insert(*slot, logical);
                self.services.image_names.insert(*slot, name.clone());
                // upload_rgba is owner-published by Runtime at the end of this
                // tick. Yield once before an instruction can reference it.
                return Ok(false);
            }
            Event::Cursor(name) if name.starts_with('-') => self.submit(
                vec![],
                vec![json!({"op":"set_cursor","cursor":{"kind":"standard","shape":"default"}})],
            )?,
            Event::Cursor(name) => {
                let Some(prepared) = self
                    .services
                    .prepare(Work::Cursor(name.clone()), self.client)?
                else {
                    return Ok(false);
                };
                let Prepared::Cursor(cursor) = prepared else {
                    bail!("Circus cursor worker type");
                };
                let frames=cursor.frames.into_iter().map(|f|json!({"width":f.image.width,"height":f.image.height,"hotspot":f.hotspot,"rgba":f.image.rgba})).collect::<Vec<_>>();
                let steps = cursor
                    .steps
                    .into_iter()
                    .map(|(i, ms)| [i as u32, ms])
                    .collect::<Vec<_>>();
                self.submit(vec![],vec![json!({"op":"define_cursor","id":0x3500_1000u32,"cursor":{"frames":frames,"steps":steps}}),json!({"op":"set_cursor","cursor":{"kind":"custom","id":0x3500_1000u32}})])?;
            }
            Event::DefineGroup { slot, group } => self.define_group(*slot, group)?,
            Event::Palette { index, rgb } => {
                self.services.palette.insert(*index, *rgb);
            }
            Event::ReadLabel { .. } => {}
            Event::FallingEffect { kind, parameter } => {
                return self.falling_effect(*kind, *parameter)
            }
            Event::BeginFade {
                out,
                duration_ms,
                color,
            } => {
                self.root()?;
                if let Some((_, target, _)) = self.services.fade.take() {
                    // Native407BE0/425E60 replace, rather than complete, the
                    // previous effect. Do not run its scene cleanup here.
                    self.submit(vec![json!({"op":"end_crossfade","id":target})], vec![])?;
                }
                let (target, source) = if *out {
                    (COLOUR_ROOT, FADE_SOURCE)
                } else {
                    (SCENE_ROOT, COLOUR_ROOT)
                };
                let mut commands = Vec::new();
                if *out {
                    let mut canvases = Vec::new();
                    if !self.services.fade_canvas {
                        canvases.push(
                            json!({"op":"create","id":FADE_CANVAS,"width":1280,"height":720}),
                        );
                    }
                    canvases.push(json!({"op":"clear","id":FADE_CANVAS,"color":[0,0,0,0]}));
                    canvases.push(json!({"op":"copy_subtree","source":SCENE_ROOT,"destination":FADE_CANVAS,"rect":[0,0,1280,720],"position":[0,0]}));
                    canvases.push(json!({"op":"present","id":FADE_CANVAS,"layer":FADE_SOURCE,"order":0,"position":[0,0]}));
                    self.host.submit_engine(
                        self.client,
                        1,
                        &serde_json::to_vec(&json!({"canvases":canvases}))?,
                    )?;
                    self.services.fade_canvas = true;
                    commands.push(json!({"op":"set_visible","id":SCENE_ROOT,"visible":false}));
                    commands.push(json!({"op":"put","layer":{"id":COLOUR_ROOT,"rect":[0,0,1280,720],"paint":{"kind":"solid","color":[color[0],color[1],color[2],255]},"order":[1,0]}}));
                }
                commands.push(json!({"op":"set_visible","id":target,"visible":true}));
                commands.push(json!({"op":"set_visible","id":source,"visible":true}));
                commands.push(json!({"op":"begin_crossfade","id":target,"source":source,"duration_ms":duration_ms,"external":true}));
                self.submit(commands, vec![])?;
                self.services.fade = Some((*out, target, source));
                self.services.fade_frames = 0;
                self.services.fade_duration_ms = *duration_ms;
            }
            Event::CenterAudioPan{kind}=>{
                let channels=match kind{0=>0..=1,1=>2..=7,2=>8..=11,_=>bail!("Circus native pan category")};
                for channel in channels {self.host.submit_audio(self.client,AudioRequest::SetPan{channel,pan:0.5,fade_ms:0.})?;}
            }
            Event::FitBackground=>{
                self.services.fit_background=true;
                if self.services.displayed.contains(&1){return self.emit(&Event::DisplayImage{slot:1});}
            }
            Event::ReleaseMusic { forced, steps } => return self.release_music(*forced, *steps),
            Event::StopAudio { channel } => self.host.submit_audio(
                self.client,
                AudioRequest::Stop {
                    channel: *channel,
                    fade_ms: 0.,
                },
            )?,
            Event::PlayAudio {
                channel,
                name,
                voice,
            } => return self.play_audio(*channel, name, *voice, false, 0, None),
            Event::PlayMusic { asset, looping } => {
                return self.play_audio(
                    0,
                    &asset.name,
                    false,
                    *looping,
                    asset.loop_start,
                    asset.loop_end,
                )
            }
            // Native positioning applies to the reusable engine Layer state.
            Event::SpritePosition { slot, position } => {
                self.services.positions.insert(*slot, *position);
                if (4..=8).contains(slot) {self.sprite_root(*slot)?;}
            }
            Event::SpriteScale{slot,percent,pivot}=>{
                self.services.sprite_scales.insert(*slot,(*percent,*pivot));self.sprite_root(*slot)?;
            }
            Event::GroupVisibility(_) => {
                bail!("Circus native group visibility mapping not yet verified")
            }
            Event::ResetScene=>{
                self.root()?;self.services.fit_background=false;
                let mut layers=vec![json!({"op":"set_visible","id":SCENE_ROOT,"visible":true})];
                if let Some((_,target,_))=self.services.fade.take(){layers.push(json!({"op":"end_crossfade","id":target}));}
                if self.services.fade_canvas{
                    layers.push(json!({"op":"set_visible","id":COLOUR_ROOT,"visible":false}));
                    self.host.submit_engine(self.client,1,&serde_json::to_vec(&json!({"canvases":[{"op":"hide","layer":FADE_SOURCE},{"op":"clear","id":FADE_CANVAS,"color":[0,0,0,0]}]}))?)?;
                }
                layers.extend(self.services.displayed.iter().map(|slot|json!({"op":"remove","id":0x3700_0000+u32::from(*slot)})));
                self.submit(layers,vec![])?;self.services.displayed.clear();self.clear_menu()?;
            }
            Event::DisplayImage { slot } => {
                self.root()?;
                let parent=if (4..=8).contains(slot){self.sprite_root(*slot)?}else{SCENE_ROOT};
                let image = self
                    .services
                    .images
                    .get(slot)
                    .context("Circus display image undefined")?;
                let position = if parent==SCENE_ROOT{self.services.positions.get(slot).copied().unwrap_or([0, 0])}else{[0,0]};
                let logical = self
                    .services
                    .image_extents
                    .get(slot)
                    .context("Circus logical image extent")?;
                let extent=if *slot==1&&self.services.fit_background{[1280,720]}else{*logical};
                self.submit(vec![json!({"op":"put","layer":{"id":0x3700_0000+u32::from(*slot),"parent":parent,"rect":[position[0],position[1],extent[0],extent[1]],"paint":{"kind":"image","asset":image.path.relative()},"sampling":{"address_u":"repeat","address_v":"repeat"},"uv":[0.,0.,logical[0] as f32/image.size[0] as f32,logical[1] as f32/image.size[1] as f32],"order":[0,slot]}})],vec![])?;
                self.services.displayed.insert(*slot);
            }
        }
        Ok(true)
    }
}
fn vm_error(e: anyhow::Error) -> crysirror_core::Error {
    crysirror_core::Error::Io(format!("{e:#}"))
}
impl VmAdapter for CircusVm {
    fn engine_id(&self) -> &'static str {
        "circus"
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
        (|| -> Result<VmYield> {
            // Seed once before resource latency. Native41032E uses a process
            // millisecond clock; the engine's virtual clock is reproducible.
            self.services.sequence_rng.get_or_insert_with(|| {
                crysirror_vfx::falling_atlas::SequenceRng::msvc(host.now_ms() as u32)
            });
            if self.services.storage.is_none() {
                self.services.storage = Some(host.storage(self.client)?);
            }
            if !self.services.system_loaded {
                let Some(Prepared::System(data)) =
                    self.services.prepare(Work::SystemLoad, self.client)?
                else {
                    return Ok(VmYield::BudgetExhausted);
                };
                self.machine.state.configured_names = data.names()?;
                self.machine.state.system_data = data;
                self.services.system_loaded = true;
            }
            let now = host.now_ms();
            let mut bridge = Bridge {
                services: &mut self.services,
                host,
                client: self.client,
                font: &self.config.font,
                input,
            };
            bridge.native_audio_update()?;
            if !bridge.effects_ready()? {
                return Ok(VmYield::BudgetExhausted);
            }
            for n in 0..budget {
                bridge.sync_audio_controls(self.machine.state.audio_controls()?)?;
                let outcome =
                    self.machine
                        .step(&mut bridge, now, n == 0 && input.buttons_pressed != 0)?;
                let native_frame = self.machine.native_frame_ready(outcome)
                    && bridge.services.loading.is_none()
                    && bridge.services.audio.is_none();
                if native_frame {
                    bridge.native_frame()?;
                    self.machine.finish_native_frame();
                }
                match outcome {
                    Outcome::Advanced => {}
                    Outcome::Pending => return Ok(VmYield::BudgetExhausted),
                    Outcome::WaitUntil(t) => {
                        return Ok(
                            if native_frame
                                && bridge.services.fade.is_none()
                                && bridge.services.effects.is_none()
                                && bridge.services.music_releases.is_empty()
                            {
                                VmYield::WaitUntil(t)
                            } else {
                                VmYield::BudgetExhausted
                            },
                        )
                    }
                    Outcome::WaitForInput => {
                        return Ok(
                            if native_frame
                                && bridge.services.fade.is_none()
                                && bridge.services.effects.is_none()
                                && bridge.services.music_releases.is_empty()
                            {
                                VmYield::WaitForInput
                            } else {
                                VmYield::BudgetExhausted
                            },
                        )
                    }
                    Outcome::Halted => return Ok(VmYield::Halted),
                }
            }
            Ok(VmYield::BudgetExhausted)
        })()
        .map_err(vm_error)
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(crysirror_core::Error::Unsupported(
            "Circus snapshots: native UI/resource safepoints pending",
        ))
    }
    fn restore(&mut self, _: &[u8]) -> crysirror_core::Result<()> {
        Err(crysirror_core::Error::Unsupported("Circus snapshots"))
    }
    fn diagnostics(&self) -> Vec<u8> {
        serde_json::to_vec(&json!({"scene":self.machine.state.scene,"pc":self.machine.state.pc,"instructions":self.machine.state.instructions,"native_groups":self.machine.state.groups.len(),"engine_images":self.services.images.len(),"worker_pending":self.services.loading.is_some(),"native_save_projection":{"gaps":self.machine.state.native_live.gaps(),"completed_opcodes":self.machine.state.native_live.completed_opcodes()},"checkpoint":self.machine.state.checkpoint.as_ref().map(|c|json!({"resume_pc":c.resume_pc,"continuation":c.continuation,"vm_bytes":c.vm_state.len(),"shared_resource_bytes":c.shared_resources.len()}))})).unwrap_or_default()
    }
}
