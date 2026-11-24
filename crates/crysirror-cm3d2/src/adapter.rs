// SPDX-License-Identifier: MPL-2.0
//! The CM3D2 VM session: game files, Unity scene state, the persistent
//! `__GameMain__` hierarchy and the scene/KAG managers ported from
//! Assembly-CSharp, presented through shared layer services.
use crate::{
    files::GameFiles,
    kag::{AdvKag, KagStep},
    ngui::{NodeState, Ui, Uploader, HEIGHT, WIDTH},
    scenes::{self, Active},
    unity_scene::{Scene, UnityData, UnityFiles},
};
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{AssetReader, ClientStorage, HostServices, InputFrame},
    state::ClientId,
    vm::{VmAdapter, VmYield},
    Error,
};
use serde::Deserialize;
use serde_json::json;
use std::{collections::HashMap, sync::Arc};

pub const ENGINE_ID: &str = "cm3d2";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Unity player data directory (`CM3D2x64_Data`).
    pub data_dir: String,
    /// Mounted `GameData/*.arc` targets in `GameUty.Init` order.
    pub archives: Vec<String>,
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.data_dir.is_empty() && !self.data_dir.contains(".."), "CM3D2 data directory");
        ensure!(!self.archives.is_empty() && self.archives.len() <= 256, "CM3D2 archive list");
        Ok(())
    }
}

/// `DateTime.Now.ToString("G", ja-JP)` of a new maid's `create_time`, in
/// UTC (the host gives no time zone).
pub fn create_time() -> String {
    let (y, m, d, h, min, sec) = civil_now();
    format!("{y}/{m:02}/{d:02} {h}:{min:02}:{sec:02}")
}

/// `DateTime.Now.ToString("yyyyMMddHHmmss")` of a save header (UTC).
pub fn save_time() -> String {
    let (y, m, d, h, min, sec) = civil_now();
    format!("{y:04}{m:02}{d:02}{h:02}{min:02}{sec:02}")
}

fn civil_now() -> (i64, i64, i64, i64, i64, i64) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (H. Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, rem / 3600, rem / 60 % 60, rem % 60)
}

/// Adapter-owned layer ids (NGUI widgets start at `ngui::LAYER_BASE`).
pub const FADE_LAYER: u32 = 1;
/// `FadeOutNoUI`: covers the 3D view below every NGUI widget.
pub const FADE3D_LAYER: u32 = 3;
const WINDOW: &str = "__GameMain__/SystemUI Root/MessageWindowPanel";

pub struct Opener {
    pub storage: Arc<dyn ClientStorage>,
    pub dir: String,
}
impl UnityFiles for Opener {
    fn open(&mut self, name: &str) -> Result<Arc<dyn AssetReader>> {
        let path = VfsPath::new(Mount::Asset, &format!("{}/{}", self.dir, name))?;
        Ok(self.storage.open_asset(&path)?)
    }
}

struct HostUploader<'a> {
    host: &'a mut dyn HostServices,
    client: ClientId,
}
impl Uploader for HostUploader<'_> {
    fn upload(&mut self, id: u32, width: u32, height: u32, rgba: &[u8]) -> Result<String> {
        Ok(self.host.upload_rgba(self.client, id, width, height, rgba)?.relative().to_owned())
    }
    fn fonts(&mut self) -> Result<Arc<dyn crysirror_core::service::FontLibrary>> {
        Ok(self.host.font_library(self.client)?)
    }
    fn measurement(&mut self) -> Result<Arc<dyn crysirror_core::service::TextMeasurement>> {
        Ok(self.host.text_measurement(self.client)?)
    }
}

/// `CameraMain.FadeIn/FadeOut`: a full-screen black cover over everything.
#[derive(Clone, Copy, Debug)]
pub struct Fade {
    pub from: f32,
    pub to: f32,
    pub start_ms: u64,
    pub duration_ms: u64,
}
impl Fade {
    pub fn value(&self, now: u64) -> f32 {
        if self.duration_ms == 0 || now >= self.start_ms + self.duration_ms {
            return self.to;
        }
        let t = (now - self.start_ms) as f32 / self.duration_ms as f32;
        self.from + (self.to - self.from) * t
    }
    pub fn done(&self, now: u64) -> bool {
        now >= self.start_ms + self.duration_ms
    }
}

/// `CameraMain` iTween animation of the orbit state.
#[derive(Clone, Debug)]
struct CameraTween {
    from: ([f32; 3], f32, [f32; 2]),
    to: ([f32; 3], f32, [f32; 2]),
    start: u64,
    ms: u64,
    ease: String,
}

/// iTween easing curves used by the scripts.
pub(crate) fn ease(name: &str, t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    match name.to_ascii_lowercase().as_str() {
        "easeoutcubic" => 1. - (1. - t).powi(3),
        "easeincubic" => t.powi(3),
        "easeinoutcubic" => if t < 0.5 { 4. * t.powi(3) } else { 1. - (-2. * t + 2.).powi(3) / 2. },
        "easeoutquad" => 1. - (1. - t) * (1. - t),
        "easeinquad" => t * t,
        "easeoutexpo" => if t >= 1. { 1. } else { 1. - 2f32.powf(-10. * t) },
        "easeinoutsine" => -((std::f32::consts::PI * t).cos() - 1.) / 2.,
        _ => t,
    }
}

/// `CharacterMgr.GetOffsetMaid`: offset by (DouPer − 50) and
/// (sintyou − 50) over `split`, towards the greater/less vectors.
pub(crate) fn maid_offset(m: &crate::chara::Maid, split: i32, per: &crate::kag::CharaPer) -> ([f32; 3], [f32; 3]) {
    let dou = m.value("DouPer") - 50;
    let shin = m.value("sintyou") - 50;
    let part = |n: i32, g: [f32; 3], l: [f32; 3]| -> [f32; 3] {
        let (v, d) = if n >= 0 { (g, split) } else { (l, -split) };
        v.map(|c| n as f32 * c / d as f32)
    };
    let add = |a: [f32; 3], b: [f32; 3]| [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
    let offset = add(part(dou, per.dou_greater, per.dou_less), part(shin, per.shin_greater, per.shin_less));
    let all = add(part(dou, per.harem_dou_greater, per.harem_dou_less), part(shin, per.harem_shin_greater, per.harem_shin_less));
    (offset, all)
}

/// A loaded level and its dynamic node state.
pub struct Level {
    pub name: String,
    pub scene: Scene,
    pub states: HashMap<usize, NodeState>,
}

pub struct Game {
    /// Built GLB models awaiting upload: (id, name, bytes).
    pub pending_models: Vec<(u32, String, Vec<u8>)>,
    pub client: ClientId,
    pub config: Config,
    pub files: GameFiles,
    pub unity: UnityData,
    pub levels: Vec<String>,
    pub main: Level,
    pub level: Option<Level>,
    pub ui: Ui,
    pub fade: Fade,
    pub fade3d: Fade,
    pub camera_control: bool,
    /// `CMSystem.MsgWndAlpha` (percent transparency).
    pub msg_wnd_alpha: f32,
    pub now: u64,
    pub active: Active,
    pub adv: AdvKag,
    /// `ScriptManager.yotogi_kag`.
    pub yotogi_kag: AdvKag,
    /// `WfCameraMoveSupport.SaveCameraPosition` of the yotogi play screen.
    pub yotogi_saved_camera: Option<([f32; 3], f32, [f32; 2])>,
    /// `MaidManagementMain.BackUpSelectMaidGUID`.
    pub maid_management_backup: Option<String>,
    /// `SystemShortcut` state.
    pub shortcut: crate::system_ui::Shortcut,
    /// Scene requests of a synchronous `LoadYotogiScript` run.
    pub yotogi_ops: Vec<crate::kag::AdvUi>,
    /// `CMSystem.ManAlpha` (percent).
    pub man_alpha: i32,
    /// The voice clip of maid 0's `AudioMan`: file, start ms, length ms, loop.
    pub voice: Option<(String, u64, u64, bool)>,
    voice_lengths: HashMap<String, u64>,
    pub pending_scene: Option<String>,
    pub input: InputFrame,
    pub log: Vec<String>,
    /// World commands queued by scene logic for this frame.
    pub world: Vec<serde_json::Value>,
    /// World entities of the current level's 3D content.
    pub level_entities: Vec<u32>,
    pub next_entity: u32,
    pub next_model: u32,
    /// `CameraMain` main camera vertical field of view.
    pub fov: f32,
    /// `LightMain` Euler rotation and intensity (white, Unity `Light`).
    pub light: ([f32; 3], f32),
    /// The active level's `RenderSettings.ambientLight` (display-referred).
    pub ambient: [f32; 3],
    pub sound: crate::sound::Sound,
    /// `CharacterMgr` stock characters (maids then men) and active slots.
    pub characters: Vec<crate::chara::Maid>,
    pub active_maids: [Option<usize>; 18],
    pub active_men: [Option<usize>; 6],
    pub(crate) chara_view: crate::chara::View,
    /// `CharacterMgr` all-position/rotation (Unity space).
    pub all_pos: [f32; 3],
    pub all_rot: [f32; 3],
    /// `m_goAllOffset.localPosition` under the all-position.
    pub all_offset: [f32; 3],
    /// `BgMgr` current background entity and name.
    pub bg: Option<(String, u32)>,
    /// The background prefab hierarchy (for `addprefabbg dest=`).
    pub bg_scene: Option<Scene>,
    /// Main camera orbit state: target, distance, around angles.
    pub camera: ([f32; 3], f32, [f32; 2]),
    /// `CameraMain.SetTargetOffset` x (screen pixels).
    pub camera_offset: f32,
    camera_anim: Option<CameraTween>,
    /// `.tex` images to upload into `UITexture.mainTexture`: (scene, node, file).
    pub pending_textures: Vec<(usize, usize, String)>,
    /// NGUI events of this frame: (scene index, node, `Class.Method`).
    pub events: Vec<(usize, usize, String)>,
    ui_presses: [Option<crate::uiev::ButtonPress>; 2],
    pub right_click: Option<(usize, usize)>,
    /// Slider being dragged: (scene index, node).
    drag: Option<(usize, usize)>,
    /// Selected `UIInput`: (scene index, node, host session).
    pub(crate) input_focus: Option<(usize, usize, crysirror_core::service::TextInputSessionHandle)>,
    /// `MotionKagManager` instances by slot.
    pub motion_kags: std::collections::BTreeMap<i32, AdvKag>,
    /// `SoundMgr` calls queued by scene logic for this frame.
    pub sound_ops: Vec<crate::sound::SoundOp>,
    /// `ScriptManager.is_motion_blend`.
    pub motion_blend: bool,
    /// `.anm` lengths (s).
    pub anm_durations: HashMap<String, f32>,
    /// NGUI `TweenAlpha` of `__GameMain__` nodes.
    pub tweens: Vec<crate::ops::Tween>,
    /// `SelectButtonCtrl` buttons and the clicked choice.
    pub choices: Vec<crate::ops::ChoiceButton>,
    pub choice_pending: Option<(String, String)>,
    /// `MessageWindowMgr` backlog: speaker, text, voice.
    pub backlog: Vec<(String, String, String)>,
    /// `CameraMain` chase camera.
    pub camera_chase: Option<Chase>,
    /// `BgMgr.m_listAttachObj`: name, entity.
    pub bg_prefabs: Vec<(String, u32)>,
    /// Entities to parent once spawned: entity, parent, parent node, transform.
    pub pending_parents: Vec<(u32, u32, Option<String>, serde_json::Value)>,
    /// `m_goAllOffset.localRotation` (Euler).
    pub all_offset_rot: [f32; 3],
    /// `ADVKagManager.skip_mode`.
    pub skip_mode: bool,
    /// `MessageWindowMgr` auto mode and backlog.
    pub msgwnd: crate::msgwnd::MessageWindow,
    /// `ConfigMgr` panel and its `CMSystem` values.
    pub cm_system: crate::config::Config,
    /// This frame's press landed on a widget other than the message box.
    pub click_on_ui: bool,
    /// `GameMain.ToApplicationQuit` requested (title End).
    pub quit_requested: bool,
    /// Last frame time (ms) for frame-rate dependent updates.
    pub last_now: u64,
    /// Decoded Unity `Resources/Texture` images by `res:<path>.tex` (PNG).
    pub res_textures: HashMap<String, Vec<u8>>,
    /// The installed `Assembly-CSharp.dll` (data tables of its static
    /// constructors).
    pub managed_asm: Option<Arc<crysirror_dotnet::Assembly>>,
}

/// `CameraChase` on a character bone.
#[derive(Clone, Debug)]
pub struct Chase {
    pub chara: usize,
    pub bone: String,
    pub offset: [f32; 3],
    pub once: bool,
    pub seconds: f32,
    pub start: u64,
    pub from: Option<[f32; 3]>,
}

impl Game {
    /// `GameMain.LoadScene`: replace the level, keeping `__GameMain__`.
    pub fn load_scene(&mut self, opener: &mut Opener, name: &str) -> Result<()> {
        let index = self
            .levels
            .iter()
            .position(|l| l.rsplit('/').next().and_then(|f| f.strip_suffix(".unity")) == Some(name))
            .with_context(|| format!("CM3D2 scene {name} is not in the build"))?;
        let file = if index == 0 { "mainData".to_owned() } else { format!("level{}", index - 1) };
        let mut scene = self.unity.scene(opener, &file)?;
        // The singleton keeps its first instance; scene copies are discarded.
        scene.roots.retain(|r| scene.nodes[*r].name != "__GameMain__");
        for id in std::mem::take(&mut self.level_entities) {
            self.world.push(json!({"op": "despawn", "id": id}));
        }
        // Level 3D content (roots outside UIRoots) becomes world entities.
        let roots: Vec<usize> = scene.roots.clone();
        for root in roots {
            if scene.nodes[root].components.iter().any(|(_, c)| matches!(c, crate::unity_scene::Component::Script { class, .. } if class == "UIRoot")) {
                continue;
            }
            let Some(model) = crate::bg::build(&mut self.unity, opener, &scene, root)? else { continue };
            self.next_model += 1;
            self.pending_models.push((self.next_model, scene.nodes[root].name.clone(), model));
        }
        if let Some(a) = self.render_settings(opener, &file)? {
            self.ambient = a;
            self.push_environment();
        }
        self.ui_presses = [None; 2];
        self.level = Some(Level { name: name.to_owned(), scene, states: HashMap::new() });
        self.active = scenes::start(self, opener, name)?;
        crate::daily::update_status_viewers(self)?;
        Ok(())
    }

    /// `RenderSettings.ambientLight` of a serialized level.
    fn render_settings(&mut self, opener: &mut Opener, file: &str) -> Result<Option<[f32; 3]>> {
        let f = self.unity.file(opener, file)?;
        let Some(info) = f.objects.iter().find(|o| o.class_id == crysirror_unity::class_id::RENDER_SETTINGS) else { return Ok(None) };
        let rs = crysirror_unity::classes::RenderSettings::read(&f.bytes(info)?)?;
        Ok(Some([rs.ambient_light[0], rs.ambient_light[1], rs.ambient_light[2]]))
    }

    /// Unity forward lighting in gamma space: no tonemapping, the white
    /// `LightMain` directional light at its intensity, the flat ambient
    /// (decoded to linear; toon materials re-encode it). Backgrounds are
    /// lightmapped in the original (SATD-cm3d2-rendering).
    fn push_environment(&mut self) {
        let d = crate::chara::unity_euler(self.light.0) * glam::Vec3::Z;
        let decode = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        self.world.push(json!({"op": "set_environment", "environment": {
            "show_sky": false, "intensity": 1.0, "bloom": 0.0, "tonemap": "none", "ambient_occlusion": 0.0,
            "ambient": self.ambient.map(|c| decode(c.clamp(0., 1.))),
            // MainDirectionalLight: soft shadows at strength 0.1 (the
            // character light is disabled in `__GameMain__`).
            "sun": {"direction": [-d.x, d.y, d.z], "color": [1.0, 1.0, 1.0], "intensity": self.light.1, "shadow_strength": 0.1}
        }}));
    }

    /// `GameObject.SetActive` on a node of the persistent `__GameMain__`.
    pub fn set_main_active(&mut self, path: &str, active: bool) -> Result<()> {
        let node = self.main.scene.find(path).with_context(|| format!("__GameMain__ node {path}"))?;
        self.main.states.entry(node).or_default().active = Some(active);
        Ok(())
    }

    /// Execute one ADV presentation request (`MessageWindowMgr`, `CameraMain`).
    pub fn adv_ui(&mut self, op: crate::kag::AdvUi, opener: &mut Opener) -> Result<()> {
        use crate::kag::AdvUi;
        match op {
            AdvUi::Window(open) => {
                self.set_main_active(WINDOW, open)?;
                if open {
                    // OpenMessageWindowPanel → DisplayBacklog(false): the
                    // message and selector viewers, not the backlog; Init
                    // leaves DragMat inactive and the scroll arrows disabled.
                    self.set_main_active(&format!("{WINDOW}/SelectorViewer"), true)?;
                    self.set_main_active(&format!("{WINDOW}/MessageViewer"), true)?;
                    self.set_main_active(&format!("{WINDOW}/BacklogViewer"), false)?;
                    if self.choices.is_empty() {
                        self.set_main_active(&format!("{WINDOW}/SelectorViewer/DragMat"), false)?;
                        self.set_main_active(&format!("{WINDOW}/SelectorViewer/Scroll Bar/Down"), false)?;
                        self.set_main_active(&format!("{WINDOW}/SelectorViewer/Scroll Bar/Up"), false)?;
                    }
                    // MessageWindowMgr: message_bg_.alpha = (100 − MsgWndAlpha) / 100,
                    // CMSystem default MsgWndAlpha 0.
                    let bg = format!("{WINDOW}/MessageViewer/MessageBox");
                    let node = self.main.scene.find(&bg).with_context(|| format!("{bg}"))?;
                    let mut color = self.main.scene.script(node, "UISprite").and_then(|f| f.floats("mColor").map(|c| [c[0], c[1], c[2], c[3]])).unwrap_or([1.; 4]);
                    color[3] = (100. - self.msg_wnd_alpha) / 100.;
                    self.main.states.entry(node).or_default().color = Some(color);
                }
            }
            AdvUi::Message { name, text } => {
                let name = self.replace_chara_name(&name);
                let text = self.replace_chara_name(&text);
                let speaker = format!("{WINDOW}/MessageViewer/SpeakerName");
                self.set_main_active(&speaker, !name.is_empty())?;
                self.set_main_text(&format!("{speaker}/Name"), &name)?;
                self.set_main_text(&format!("{WINDOW}/MessageViewer/Message"), &text)?;
                // MessageWindowMgr.AddBackLog / SetText (voice by TalkVoice).
                self.backlog.push((name, text, String::new()));
                if self.backlog.len() > crate::msgwnd::BACKLOG_UNITS {
                    self.backlog.remove(0);
                }
                let voice = self.main.scene.find(&format!("{WINDOW}/MessageViewer/Buttons/Voice"));
                if let Some(v) = voice {
                    self.main.states.entry(v).or_default().enabled = Some(false);
                }
            }
            AdvUi::ClearText => {
                self.set_main_text(&format!("{WINDOW}/MessageViewer/Message"), "")?;
                self.set_main_text(&format!("{WINDOW}/MessageViewer/SpeakerName/Name"), "")?;
            }
            AdvUi::Fade { to, seconds } => self.fade_to(to, seconds),
            AdvUi::Fade3d { to, seconds } => {
                let from = self.fade3d.value(self.now);
                self.fade3d = Fade { from, to, start_ms: self.now, duration_ms: (seconds.max(0.) * 1000.) as u64 };
            }
            AdvUi::CameraControl(on) => self.camera_control = on,
            AdvUi::SetBg(name) => {
                if let Some((_, id)) = self.bg.take() {
                    self.world.push(json!({"op": "despawn", "id": id}));
                }
                // BgMgr.ChangeBg drops the attached prefabs with the old BG.
                for (_, id) in std::mem::take(&mut self.bg_prefabs) {
                    self.world.push(json!({"op": "despawn", "id": id}));
                }
                self.bg_scene = None;
                if !name.is_empty() {
                    let go = self
                        .unity
                        .resource_of(opener, &format!("BG/{name}"), crysirror_unity::class_id::GAME_OBJECT)?
                        .with_context(|| format!("BG prefab BG/{name}"))?;
                    let scene = self.unity.prefab(opener, &go)?;
                    if let Some(glb) = crate::bg::build(&mut self.unity, opener, &scene, scene.roots[0])? {
                        self.next_model += 1;
                        self.next_entity += 1;
                        let id = 0x100 + self.next_entity;
                        self.pending_models.push((self.next_model, format!("bg:{id}"), glb));
                        self.bg = Some((name, id));
                    }
                    self.bg_scene = Some(scene);
                }
            }
            AdvUi::CharaVisible { man, slot, visible } => {
                if let Some(c) = if man { self.man(slot) } else { self.maid(slot) } {
                    c.visible = visible;
                }
            }
            AdvUi::CharaVisibleAllOff { man } => {
                for c in self.characters.iter_mut().filter(|c| c.man == man) {
                    c.visible = false;
                }
            }
            AdvUi::CharaActivate { man, slot, npc, stock } => match npc {
                Some(name) => self.activate_npc(man, slot, &name)?,
                None => {
                    let stock = stock.unwrap_or(0);
                    let index = self.characters.iter().enumerate().filter(|(_, c)| c.is_stock() && c.man == man).nth(stock).map(|(i, _)| i);
                    let slots: &mut [Option<usize>] = if man { &mut self.active_men } else { &mut self.active_maids };
                    *slots.get_mut(slot).context("character slot")? = index;
                }
            },
            AdvUi::CharaDeactivate { man, slot } => {
                if let Some(c) = if man { self.man(slot) } else { self.maid(slot) } {
                    c.visible = false;
                }
                let slots: &mut [Option<usize>] = if man { &mut self.active_men } else { &mut self.active_maids };
                if let Some(s) = slots.get_mut(slot) {
                    *s = None;
                }
            }
            AdvUi::Camera { target, distance, around } => {
                self.camera_anim = None;
                self.camera = (target, distance, around);
            }
            AdvUi::CameraReset => {
                self.camera_anim = None;
                self.camera = ([0., 1.6, 0.], 3., [180., 0.]);
            }
            AdvUi::CameraAnimation { from, to, delay_ms, ms, ease } => {
                let from = from.unwrap_or(self.camera);
                self.camera = from;
                self.camera_anim = Some(CameraTween { from, to, start: self.now + delay_ms, ms, ease });
            }
            AdvUi::LightMain { rotation, intensity } => {
                self.light = (rotation.unwrap_or([50., 0., 0.]), intensity.unwrap_or(0.95));
                self.push_environment();
            }
            // ScriptManager.LoadMotionScript(slot, false, file, label): the
            // slot's manager is reused; it is enabled and executes.
            AdvUi::MotionScript { slot, file, label } => {
                let storage = opener.storage.clone();
                let mut kag = self.motion_kags.remove(&slot).unwrap_or_else(|| AdvKag::motion(self.adv.tjs.clone()));
                kag.enabled = true;
                kag.load(&self.files, storage.as_ref(), &file, &label)?;
                kag.next_motion = false;
                kag.exec();
                // A motion script runs to its [s] within the same frame.
                kag.update(self.now, false, false, &self.files, storage.as_ref())?;
                let ops = std::mem::take(&mut kag.ui);
                self.motion_kags.insert(slot, kag);
                for op in ops {
                    self.adv_ui(op, opener)?;
                }
            }
            AdvUi::AllPos { position, rotation } => {
                if let Some(p) = position {
                    self.all_pos = p;
                }
                if let Some(r) = rotation {
                    self.all_rot = r;
                }
            }
            AdvUi::Offset { man, slot, position, rotation } => {
                if let Some(c) = if man { self.man(slot) } else { self.maid(slot) } {
                    if let Some(p) = position {
                        c.position = p;
                    }
                    if let Some(r) = rotation {
                        c.rotation = r;
                    }
                }
            }
            AdvUi::MaidOffset { list, split, multi } => {
                let mut harem = [0f32; 3];
                for (slot, per) in list {
                    let Some(m) = self.maid(slot) else { continue };
                    let (offset, all) = maid_offset(m, split, &per);
                    m.position = offset;
                    if all[1] > harem[1] {
                        harem = all;
                    }
                }
                if multi {
                    // m_goAllOffset.localPosition = (0, max y, 0).
                    self.all_offset = [0., harem[1], 0.];
                }
            }
            AdvUi::ManOffset { man, target, split, per } => {
                let target = if target.0 { self.man(target.1).cloned() } else { self.maid(target.1).cloned() };
                if let (Some(t), Some(m)) = (target, self.man(man)) {
                    m.position = maid_offset(&t, split, &per).0;
                }
            }
            AdvUi::Face { man, slot, name } => {
                if let Some(c) = if man { self.man(slot) } else { self.maid(slot) } {
                    let sets = c.blend_sets.clone();
                    c.face.anime(&name, &sets);
                }
            }
            other => self.adv_ui_ext(other, opener)?,
        }
        Ok(())
    }

    /// The camera of this frame (orbit state, or its tween).
    fn camera_command(&mut self) -> serde_json::Value {
        if let Some(t) = &self.camera_anim {
            let f = if t.ms == 0 { 1. } else { ease(&t.ease, (self.now.saturating_sub(t.start)) as f32 / t.ms as f32) };
            let lerp = |a: f32, b: f32| a + (b - a) * f;
            self.camera = (
                [lerp(t.from.0[0], t.to.0[0]), lerp(t.from.0[1], t.to.0[1]), lerp(t.from.0[2], t.to.0[2])],
                lerp(t.from.1, t.to.1),
                [lerp(t.from.2[0], t.to.2[0]), lerp(t.from.2[1], t.to.2[1])],
            );
            if self.now >= t.start + t.ms {
                self.camera_anim = None;
            }
        }
        let mut c = crate::bg::orbit_camera_offset(self.camera.0, self.camera.2, self.camera.1, self.fov, self.camera_offset);
        c["op"] = json!("set_camera");
        c
    }

    /// `ScriptManager.ReplaceCharaName` with the current player/maid names.
    pub fn replace_chara_name(&self, text: &str) -> String {
        let Ok(tjs) = self.adv.tjs.lock() else { return text.to_owned() };
        let state = &tjs.runtime.host().state;
        // [HF]: first name when is_first_name_call, else last name.
        let (hf, hl) = state
            .maid(0)
            .map(|m| (if m.is_first_name_call { m.first_name.clone() } else { m.last_name.clone() }, m.last_name.clone()))
            .unwrap_or_default();
        text.replace("[SF]", &state.player.player_name).replace("[HF]", &hf).replace("[HL]", &hl)
    }

    pub fn set_main_text(&mut self, path: &str, text: &str) -> Result<()> {
        let node = self.main.scene.find(path).with_context(|| format!("__GameMain__ node {path}"))?;
        self.main.states.entry(node).or_default().text = Some(text.to_owned());
        Ok(())
    }

    /// `ProfileCtrl.UpdateProfileData` for active maid `slot`: features,
    /// propensities and classes while not employed, then the body figures
    /// (`UpdateProfileComment` → `UpdateBodyParam`).
    pub fn update_profile_data(&mut self, slot: usize) -> Result<()> {
        let Some(i) = self.active_maids.get(slot).copied().flatten() else { return Ok(()) };
        let props: HashMap<&str, f32> = self.characters[i].props.iter().enumerate().filter_map(|(k, p)| Some((*crate::chara::MPN.get(k)?, p.value as f32))).collect();
        let mut tjs = self.adv.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let state = &mut tjs.runtime.host_mut().state;
        let data = state.data()?;
        let (maids, player) = (&mut state.maids, &mut state.player);
        let m = maids.entry(i).or_default();
        let mut p = crate::maid_param::MaidParam { st: m, data: &data };
        if !p.st.employment {
            p.update_feature()?;
            p.update_propensity()?;
            p.init_class_types(&crate::maid_param::PlayerParam { st: player, data: &data })?;
        }
        // SATD-cm3d2-game-logic: the profile text of UpdateProfileComment.
        p.update_body(&|mpn| props.get(mpn).copied().unwrap_or(0.));
        Ok(())
    }

    /// `SystemDialog.Show(text, TYPE.OK, onOk)`: `handler` runs on OK.
    pub fn show_system_dialog(&mut self, text: &str, handler: &str) -> Result<()> {
        const DLG: &str = "__GameMain__/SystemUI Root/SystemDialog";
        self.set_main_active(DLG, true)?;
        self.set_main_active(&format!("{DLG}/Base/Ok"), true)?;
        self.set_main_active(&format!("{DLG}/Base/Cancel"), false)?;
        self.set_main_text(&format!("{DLG}/Base/Message"), text)?;
        if let Some(n) = self.main.scene.find(&format!("{DLG}/Base/Ok")) {
            self.main.states.entry(n).or_default().click = Some(handler.into());
        }
        Ok(())
    }

    /// Character-manager state that `ScriptManager`'s TJS functions read.
    pub fn mirror_tjs(&mut self) -> Result<()> {
        let mut tjs = self.adv.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let state = &mut tjs.runtime.host_mut().state;
        state.active_maids = self.active_maids.to_vec();
        state.stock = self.characters.iter().enumerate().filter(|(_, c)| !c.man && c.is_stock()).map(|(i, _)| i).collect();
        state.next_character = self.characters.len();
        state.bg_name = self.bg.as_ref().map(|(n, _)| n.clone()).unwrap_or_default();
        for (i, c) in self.characters.iter().enumerate() {
            if c.man {
                continue;
            }
            let body = state.bodies.entry(i).or_default();
            body.props.clear();
            for (k, p) in c.props.iter().enumerate() {
                if let Some(name) = crate::chara::MPN.get(k) {
                    body.props.insert((*name).to_owned(), (p.kind, p.value));
                }
            }
            body.loaded_slots = c.slots.iter().filter(|(_, s)| s.model.is_some()).map(|(n, _)| (*n).to_owned()).collect();
            body.params = c.slots.values().map(|s| s.params.clone()).collect();
        }
        Ok(())
    }

    /// Character-manager work queued by TJS functions.
    pub fn run_tjs_requests(&mut self, opener: &mut Opener) -> Result<()> {
        let requests = {
            let mut tjs = self.adv.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
            std::mem::take(&mut tjs.runtime.host_mut().state.requests)
        };
        for r in requests {
            match r {
                crate::tjs::Request::CreateMaid(index) => {
                    // TJSFuncCreateNewMaid: StopMotionScript, AddStockMaid,
                    // SetActiveMaid(maid, 0), Visible, AllProcPropSeqStart.
                    self.stop_motion_scripts();
                    anyhow::ensure!(index == self.characters.len(), "CreateNewMaid index {index} ≠ {}", self.characters.len());
                    let storage = opener.storage.clone();
                    let mut maid = crate::chara::Maid::new(false);
                    maid.set_up_model(&self.files, storage.as_ref(), "_I_default.menu")?;
                    maid.visible = true;
                    let stand = self.anm_duration(opener, "maid_stand01.anm")?;
                    maid.cross_fade("maid_stand01.anm", true, false, 0.5, self.now, stand);
                    let sets = maid.blend_sets.clone();
                    maid.face.anime("通常", &sets);
                    self.characters.push(maid);
                    self.active_maids[0] = Some(index);
                }
                crate::tjs::Request::SetActiveMaid { slot, chara, visible } => {
                    self.stop_motion_scripts();
                    *self.active_maids.get_mut(slot).context("maid slot")? = Some(chara);
                    self.characters.get_mut(chara).context("SetActiveMaid character")?.visible = visible;
                }
                crate::tjs::Request::RefreshMaid(slot) => {
                    let id = self.active_maids.get(slot).copied().flatten().context("empty refresh slot")?;
                    self.characters[id].all_proc_prop(&self.files,opener.storage.as_ref())?;
                }
                crate::tjs::Request::DeactivateMaid(slot) => {
                    if let Some(index) = self.active_maids.get_mut(slot).and_then(Option::take) {
                        self.characters[index].visible = false;
                    }
                }
                crate::tjs::Request::ClearCallStack => self.adv.clear_call_stack(),
                crate::tjs::Request::TrophyEffect(id) => {
                    // SATD-cm3d2-ngui: TrophyAchieveEffect is not drawn yet.
                    self.log.push(format!("trophy {id} achieved"));
                }
            }
        }
        Ok(())
    }

    /// `CharacterMgr.LoadDefault`: stock maid 0 in active slot 0, seven NPC
    /// maids from their `Resources/Preset` presets, stock men 0..3 in active
    /// man slots and the NPC man (`_I_man_oji.menu`), all but maid 0 hidden.
    pub fn load_default_characters(&mut self, opener: &mut Opener) -> Result<()> {
        if !self.characters.is_empty() {
            return Ok(());
        }
        let storage = opener.storage.clone();
        let mut maid = crate::chara::Maid::new(false);
        maid.set_up_model(&self.files, storage.as_ref(), "_I_default.menu")?;
        maid.visible = true;
        // Maid.LateUpdate after loading: maid_stand01, face 通常.
        // Maid.LateUpdate after loading: CrossFade(maid_stand01, loop, 0.5), FaceAnime(通常).
        let stand = self.anm_duration(opener, "maid_stand01.anm")?;
        maid.cross_fade("maid_stand01.anm", true, false, 0.5, self.now, stand);
        let sets = maid.blend_sets.clone();
        maid.face.anime("通常", &sets);
        self.characters.push(maid);
        self.active_maids[0] = Some(0);
        let mut tjs = self.adv.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let state = &mut tjs.runtime.host_mut().state;
        let now = crate::adapter::create_time();
        state.init_maid(0, None, now.clone())?;
        drop(tjs);
        // CharacterMgr.strMaidNameToSlotNo / strMaidNpcPresetFile [5..12).
        let npc_maids = [
            ("メイド秘書", "pre_maid_secretary"), ("よし子", "pre_maid_secretary"), ("安祐美", "pre_maid_secretary"),
            ("桜", "pre_maid_secretary"), ("藍依", "pre_maid_secretary"), ("叔父メイド", "pre_uncles_maid"),
            ("エステメイド", "pre_esthe_maid"),
        ];
        for (name, preset) in npc_maids {
            let mut npc = crate::chara::Maid::new(false);
            npc.set_up_model(&self.files, storage.as_ref(), "_I_default.menu")?;
            let key = self
                .unity
                .resource_of(opener, &format!("Preset/{preset}"), crysirror_unity::class_id::TEXT_ASSET)?
                .with_context(|| format!("Resources Preset/{preset}"))?;
            let (_, bytes) = self.unity.bytes(opener, &key)?;
            let (_, data) = crysirror_unity::classes::text_asset(&bytes)?;
            crate::preset::Preset::parse(&data).with_context(|| format!("preset {preset}"))?.apply(&mut npc);
            npc.all_proc_prop(&self.files, storage.as_ref())?;
            npc.name_last = name.into();
            npc.npc = true;
            {
                let mut tjs = self.adv.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
                tjs.runtime.host_mut().state.init_maid(self.characters.len(), Some(name), crate::adapter::create_time())?;
            }
            npc.cross_fade("maid_stand01.anm", true, false, 0.5, self.now, stand);
            let sets = npc.blend_sets.clone();
            npc.face.anime("通常", &sets);
            self.characters.push(npc);
        }
        for i in 0..4 {
            let mut man = crate::chara::Maid::new(true);
            man.set_up_model(&self.files, storage.as_ref(), "_I_man.menu")?;
            self.characters.push(man);
            self.active_men[i] = Some(self.characters.len() - 1);
        }
        let mut oji = crate::chara::Maid::new(true);
        oji.set_up_model(&self.files, storage.as_ref(), "_I_man.menu")?;
        oji.set_up_model(&self.files, storage.as_ref(), "_I_man_oji.menu")?;
        oji.name_last = "叔父".into();
        oji.npc = true;
        self.characters.push(oji);
        Ok(())
    }

    /// `CharacterMgr.CharaActivate` with `npc=`: activate the named NPC.
    pub fn activate_npc(&mut self, man: bool, slot: usize, name: &str) -> Result<()> {
        let index = self
            .characters
            .iter()
            .position(|c| c.npc && c.man == man && c.name_last == name)
            .with_context(|| format!("NPC {name} is not a stock character"))?;
        let slots: &mut [Option<usize>] = if man { &mut self.active_men } else { &mut self.active_maids };
        *slots.get_mut(slot).context("character slot")? = Some(index);
        Ok(())
    }

    pub fn scene_mut(&mut self, index: usize) -> Option<&mut Level> {
        if index == 0 { Some(&mut self.main) } else { self.level.as_mut() }
    }

    /// `UICamera`: press, drag and text delivery of this frame.
    fn pointer_events(&mut self, host: &mut dyn HostServices, client: ClientId) -> Result<()> {
        use crysirror_core::service::buttons as b;
        let (x, y) = (self.input.pointer_x as f32, self.input.pointer_y as f32);
        // UICamera sends OnClick for the right button too.
        let pressed = self.input.buttons_pressed & (b::MOUSE_LEFT | b::MOUSE_RIGHT) != 0;
        let down = self.input.buttons_down & b::MOUSE_LEFT != 0;
        self.click_on_ui = false;
        let release_buttons = self.level.as_ref().is_some_and(|l| matches!(l.name.as_str(), "SceneTitle" | "SceneDaily"));
        self.right_click = None;
        let picked = crate::uiev::pick(&self.ui.hits, x, y);
        if pressed {
            if let Some((scene, node, handle)) = self.input_focus {
                if picked.map(|hit| (hit.scene, hit.node)) != Some((scene, node)) {
                    host.release_text_input_session(client, handle)?;
                    self.input_focus = None;
                }
            }
            let path = picked.and_then(|h| self.scene_mut(h.scene).map(|l| l.scene.path(h.node)));
            self.log.push(format!("press {x},{y}: {:?} {path:?}", picked.map(|h| (h.scene, h.node, h.order))));
            if let Some(hit) = picked {
                let level = self.scene_mut(hit.scene).context("hit scene")?;
                let slider = crate::uiev::slider_of(&level.scene, hit.node);
                let is_input = level.scene.script(hit.node, "UIInput").is_some();
                let handlers = crate::uiev::click_handlers(&level.scene, &level.states, hit.node);
                if let Some(s) = slider {
                    self.drag = Some((hit.scene, s));
                } else if is_input && self.input_focus.is_none() {
                    let h = host.open_text_input_session(client, crysirror_core::service::TextInputSessionSpec { capture: true, ..Default::default() })?;
                    self.input_focus = Some((hit.scene, hit.node, h));
                }
                // UICamera.isFallThrough is false over any widget; the
                // message box forwards its click (ClickMessageBox).
                self.click_on_ui = !handlers.iter().any(|h| h == "MessageWindowMgr.ClickMessageBox");
                if !release_buttons {
                    self.events.extend(handlers.into_iter().map(|h| (hit.scene, hit.node, h)));
                }
            }
        }
        if release_buttons {
            let target = picked.map(|h| (h.scene,h.node));
            for (index, bit) in [b::MOUSE_LEFT,b::MOUSE_RIGHT].into_iter().enumerate() {
                let clicked = crate::uiev::release_click(&mut self.ui_presses[index], target, [x,y],
                    self.input.buttons_pressed & bit != 0, self.input.buttons_down & bit != 0);
                if let Some((si,node)) = clicked {
                    if bit == b::MOUSE_RIGHT { self.right_click = Some((si, node)); }
                    let level = self.scene_mut(si).context("clicked scene")?;
                    let handlers = crate::uiev::click_handlers(&level.scene, &level.states, node);
                    self.events.extend(handlers.into_iter().map(|h| (si,node,h)));
                }
            }
        }
        self.ui.button_visual = picked.filter(|_| release_buttons).and_then(|hit| {
            let pressed = self.ui_presses.iter().flatten().any(|press| press.target == (hit.scene, hit.node));
            let level = self.scene_mut(hit.scene)?;
            let button = level.scene.script(hit.node, "UIButton")?;
            let target = button.pptr("tweenTarget").filter(|reference| !reference.is_null())
                .and_then(|reference| level.scene.find_ref(hit.node, reference.path_id)).unwrap_or(hit.node);
            let color = button.floats(if pressed { "pressed" } else { "hover" })?;
            let sprite = button.string(if pressed { "pressedSprite" } else { "hoverSprite" })
                .filter(|s| !s.is_empty()).map(str::to_owned);
            Some(crate::ngui::ButtonVisual { target: (hit.scene, target), color: color.try_into().ok()?, sprite })
        });
        if !down {
            self.drag = None;
        }
        if let Some((si, node)) = self.drag {
            let rect = self.ui.rects.get(&(si, node)).copied();
            let level = self.scene_mut(si).context("drag scene")?;
            let fields = level.scene.script(node, "UISlider").cloned().unwrap_or_default();
            if let Some(rect) = rect {
                let v = crate::uiev::slider_value(rect, x, &fields);
                if level.states.get(&node).and_then(|s| s.value) != Some(v) {
                    level.states.entry(node).or_default().value = Some(v);
                    // UIProgressBar: the thumb sits at the value along the bar.
                    let bar = level.scene.script(node, "UISprite").or_else(|| level.scene.script(node, "UIWidget"));
                    let (w, px) = bar.map(|f| (f.int("mWidth").unwrap_or(0) as f32, crate::ngui::pivot_x(f.int("mPivot").unwrap_or(4)))).unwrap_or((0., 0.5));
                    if let Some(thumb) = fields.pptr("thumb").and_then(|t| level.scene.find_ref(node, t.path_id)) {
                        let p = level.scene.nodes[thumb].position;
                        level.states.entry(thumb).or_default().position = Some([(v - px) * w, p[1], p[2]]);
                    }
                    // EventDelegate.Add(onChange) at run time, then the serialized list.
                    let added = level.states.get(&node).and_then(|s| s.click.clone());
                    for h in added.into_iter().chain(crate::uiev::delegates(&level.scene, fields.array("onChange"))) {
                        self.events.push((si, node, h));
                    }
                }
            }
        }
        if let Some((si, node, h)) = self.input_focus {
            let text = host.take_text_input_committed(client, h)?;
            if !text.is_empty() {
                let level = self.scene_mut(si).context("input scene")?;
                let label = level.scene.child(node, "Label");
                let current = level.states.get(&node).and_then(|s| s.text.clone()).unwrap_or_default();
                let mut value = current;
                for c in text.chars() {
                    match c {
                        '\u{8}' => {
                            value.pop();
                        }
                        '\r' | '\n' => {}
                        c => value.push(c),
                    }
                }
                level.states.entry(node).or_default().text = Some(value.clone());
                if let Some(l) = label {
                    level.states.entry(l).or_default().text = Some(value);
                }
                let f = level.scene.script(node, "UIInput").cloned().unwrap_or_default();
                for h in crate::uiev::delegates(&level.scene, f.array("onChange")) {
                    self.events.push((si, node, h));
                }
            }
        }
        if self.input.buttons_pressed & b::ESCAPE != 0 {
            if let Some((_, _, h)) = self.input_focus.take() {
                host.release_text_input_session(client,h)?;
            }
        }
        Ok(())
    }

    /// `CharacterMgr.VisibleAll`.
    pub fn visible_all(&mut self, visible: bool) {
        for c in &mut self.characters {
            c.visible = visible;
        }
    }

    /// `CharacterMgr.GetMaid(n)`.
    pub fn maid(&mut self, n: usize) -> Option<&mut crate::chara::Maid> {
        let i = (*self.active_maids.get(n)?)?;
        self.characters.get_mut(i)
    }
    pub fn man(&mut self, n: usize) -> Option<&mut crate::chara::Maid> {
        let i = (*self.active_men.get(n)?)?;
        self.characters.get_mut(i)
    }

    /// `SoundMgr.PlaySystem(SeType)`.
    pub fn play_system(&mut self, se: crate::sound::SeType) {
        self.sound_ops.push(crate::sound::SoundOp::Play {
            kind: crate::sound::Kind::System,
            file: crate::sound::system_file(se).into(),
            fade_s: 0.,
            looping: false,
        });
    }

    pub fn fade_to(&mut self, to: f32, seconds: f32) {
        let from = self.fade.value(self.now);
        self.fade = Fade { from, to, start_ms: self.now, duration_ms: (seconds.max(0.) * 1000.) as u64 };
    }

    /// Unity `Input.GetMouseButtonUp(0..2) || GetKeyUp(Return)`, sampled as
    /// the logical press edge of this frame.
    /// `AudioSourceMgr.GetLength` of a voice file (ms), cached.
    pub fn voice_length(&mut self, storage: &dyn ClientStorage, file: &str) -> Result<u64> {
        if let Some(l) = self.voice_lengths.get(file) {
            return Ok(*l);
        }
        let bytes: std::sync::Arc<[u8]> = self.files.read(storage, file)?.into();
        let source = crysirror_audio::decoder::open_memory(bytes, Some("ogg")).with_context(|| format!("voice {file}"))?;
        let rate = source.sample_rate().max(1) as u64;
        let len = source.frame_count().map_or(0, |f| f * 1000 / rate);
        self.voice_lengths.insert(file.to_owned(), len);
        Ok(len)
    }

    /// `AudioSourceMgr.isPlay` of maid 0's voice.
    pub fn voice_playing(&self) -> bool {
        self.voice.as_ref().is_some_and(|v| v.3 || self.now < v.1 + v.2)
    }

    /// `CharacterMgr.ManAlphaUpdate`.
    pub fn man_alpha_update(&mut self) {
        let a = self.man_alpha.clamp(0, 100) as f32 / 100.;
        for c in self.characters.iter_mut().filter(|c| c.man) {
            if c.man_alpha != a {
                c.man_alpha = a;
                for s in c.slots.values_mut() {
                    s.dirty = true;
                }
            }
        }
    }

    pub fn any_click(&self) -> bool {
        use crysirror_core::service::buttons as b;
        self.input.buttons_pressed & (b::MOUSE_LEFT | b::MOUSE_RIGHT | b::MOUSE_MIDDLE | b::ENTER) != 0
    }
}

pub struct Cm3d2Vm {
    client: ClientId,
    config: Config,
    game: Option<Game>,
    error: Option<String>,
}

impl Cm3d2Vm {
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        Ok(Self { client, config, game: None, error: None })
    }

    fn boot(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let storage = host.storage(self.client)?;
        let mut opener = Opener { storage: storage.clone(), dir: self.config.data_dir.clone() };
        let files = GameFiles::index(storage.as_ref(), &self.config.archives)?;
        let mut unity = UnityData::new()?;
        let levels = {
            let main = unity.file(&mut opener, "mainData")?;
            let info = *main
                .objects
                .iter()
                .find(|o| o.class_id == crysirror_unity::class_id::BUILD_SETTINGS)
                .context("CM3D2 mainData has no BuildSettings")?;
            crysirror_unity::classes::build_levels(&main.bytes(&info)?)?
        };
        let mut main_scene = unity.scene(&mut opener, "mainData")?;
        // Build index 0 holds the persistent root and the first scene's root.
        let first: Vec<usize> = main_scene.roots.iter().copied().filter(|r| main_scene.nodes[*r].name != "__GameMain__").collect();
        main_scene.roots.retain(|r| main_scene.nodes[*r].name == "__GameMain__");
        ensure!(!main_scene.roots.is_empty(), "CM3D2 mainData has no __GameMain__");
        let mut first_scene = main_scene.clone();
        first_scene.roots = first;
        let adv = AdvKag::new(&files, storage.as_ref())?;
        // Game tables (`.nei`) with the installed plug-in's key, plus the
        // MaidRandomName arrays of the installed Assembly-CSharp.
        let dll = {
            let reader = opener.open("Plugins/cm3d2_x64.dll")?;
            let len = reader.len()? as usize;
            anyhow::ensure!(len <= 64 << 20, "cm3d2_x64.dll too large");
            let mut bytes = vec![0u8; len];
            let mut at = 0;
            while at < len {
                let n = reader.read_at(at as u64, &mut bytes[at..])?;
                anyhow::ensure!(n > 0, "short read of cm3d2_x64.dll");
                at += n;
            }
            bytes
        };
        let keys = crate::native_key::NativeKeys::from_dll(&dll)?;
        let mut data = crate::gamedata::GameData::load(&files, storage.as_ref(), &keys.table_key)?;
        let mut game = Game {
            client: self.client,
            config: self.config.clone(),
            files,
            unity,
            levels,
            main: Level { name: "__GameMain__".into(), scene: main_scene, states: HashMap::new() },
            level: Some(Level { name: "_cm3d2".into(), scene: first_scene, states: HashMap::new() }),
            ui: Ui::default(),
            fade: Fade { from: 1., to: 1., start_ms: 0, duration_ms: 0 },
            fade3d: Fade { from: 0., to: 0., start_ms: 0, duration_ms: 0 },
            camera_control: false,
            msg_wnd_alpha: 0.,
            now: 0,
            active: Active::First,
            yotogi_kag: AdvKag::yotogi(adv.tjs.clone()),
            yotogi_saved_camera: None,
            yotogi_ops: Vec::new(),
            shortcut: Default::default(),
            maid_management_backup: None,
            man_alpha: 50,
            voice: None,
            voice_lengths: HashMap::new(),
            adv,
            pending_scene: None,
            input: InputFrame::default(),
            log: Vec::new(),
            world: Vec::new(),
            level_entities: Vec::new(),
            next_entity: 0,
            next_model: 0,
            fov: 35.,
            light: ([50., 0., 0.], 0.95),
            ambient: [0.; 3],
            pending_models: Vec::new(),
            sound: crate::sound::Sound::new(),
            characters: Vec::new(),
            active_maids: [None; 18],
            active_men: [None; 6],
            chara_view: crate::chara::View::default(),
            all_pos: [0.; 3],
            all_rot: [0.; 3],
            all_offset: [0.; 3],
            bg: None,
            bg_scene: None,
            camera: ([0., 1.6, 0.], 3., [180., 0.]),
            camera_anim: None,
            camera_offset: 0.,
            motion_kags: std::collections::BTreeMap::new(),
            events: Vec::new(),
            ui_presses: [None; 2],
            right_click: None,
            pending_textures: Vec::new(),
            drag: None,
            input_focus: None,
            sound_ops: Vec::new(),
            motion_blend: true,
            anm_durations: HashMap::new(),
            tweens: Vec::new(),
            choices: Vec::new(),
            choice_pending: None,
            backlog: Vec::new(),
            camera_chase: None,
            bg_prefabs: Vec::new(),
            pending_parents: Vec::new(),
            all_offset_rot: [0.; 3],
            skip_mode: false,
            msgwnd: Default::default(),
            cm_system: Default::default(),
            click_on_ui: false,
            quit_requested: false,
            last_now: 0,
            res_textures: HashMap::new(),
            managed_asm: None,
        };
        if let Some(cam) = game.main.scene.find("__GameMain__/Camera/CameraMain/CameraMain") {
            if let Some(c) = game.main.scene.nodes[cam].components.iter().find_map(|(_, c)| match c {
                crate::unity_scene::Component::Camera(c) => Some(c.fov),
                _ => None,
            }) {
                game.fov = c;
            }
        }
        game.ambient = game.render_settings(&mut opener, "mainData")?.unwrap_or_default();
        game.push_environment();
        // CMSystem.LoadIni defaults: ViewFps = false.
        game.set_main_active("__GameMain__/SystemUI Root/FpsCounter", false)?;
        game.active = scenes::start(&mut game, &mut opener, "_cm3d2")?;
        host.submit_engine(
            self.client,
            1,
            &serde_json::to_vec(&json!({"layers": [
                {"op": "put", "layer": {"id": FADE3D_LAYER, "rect": [0, 0, WIDTH, HEIGHT], "paint": {"kind": "solid", "color": [0, 0, 0, 255]}, "opacity": 0, "order": [-100000, 0]}},
                {"op": "put", "layer": {"id": FADE_LAYER, "rect": [0, 0, WIDTH, HEIGHT], "paint": {"kind": "solid", "color": [0, 0, 0, 255]}, "order": [100000, 0]}},
            ]}))?,
        )?;
        let asm = game.managed(&mut opener)?;
        data.set_random_names(&asm)?;
        {
            let data = std::sync::Arc::new(data);
            let mut tjs = game.adv.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let state = &mut tjs.runtime.host_mut().state;
            state.seed(host.now_ms() ^ 0x5eed_cafe);
            state.data = Some(data.clone());
            // CharacterMgr's PlayerParam of a new game.
            crate::maid_param::PlayerParam { st: &mut state.player, data: &data }.init();
        }
        self.game = Some(game);
        Ok(())
    }

    fn step_inner(&mut self, host: &mut dyn HostServices, input: InputFrame) -> Result<VmYield> {
        if let Some(e) = &self.error {
            bail!("{e}");
        }
        if self.game.is_none() {
            self.boot(host)?;
        }
        let storage = host.storage(self.client)?;
        let mut opener = Opener { storage: storage.clone(), dir: self.config.data_dir.clone() };
        let game = self.game.as_mut().unwrap();
        game.now = host.now_ms();
        game.input = input;
        game.pointer_events(host, self.client)?;
        let quit = game.system_shortcut(&mut opener)?;
        game.window_events(&mut opener)?;
        // CharacterMgr active slots as seen by ScriptManager's TJS functions.
        game.mirror_tjs()?;
        scenes::update(game, &mut opener)?;
        game.run_tjs_requests(&mut opener)?;
        // NGUI events left by the scene: the message window's choices.
        for (si, node, handler) in std::mem::take(&mut game.events) {
            if !(si == 0 && game.choice_event(&mut opener, node, &handler)?) {
                game.log.push(format!("NGUI event {handler} not handled"));
            }
        }
        game.update_tweens();
        let fading = !game.fade.done(game.now) || !game.fade3d.done(game.now);
        let click = game.adv_click(&mut opener)?;
        let step = game.adv.update(game.now, click, fading, &game.files, storage.as_ref())?;
        game.run_tjs_requests(&mut opener)?;
        for op in std::mem::take(&mut game.adv.ui) {
            game.adv_ui(op, &mut opener)?;
        }
        // Motion KAG managers resume their waits each frame.
        let slots: Vec<i32> = game.motion_kags.keys().copied().collect();
        for slot in slots {
            let Some(mut kag) = game.motion_kags.remove(&slot) else { continue };
            kag.update(game.now, false, false, &game.files, storage.as_ref())?;
            let ops = std::mem::take(&mut kag.ui);
            game.sound_ops.append(&mut kag.sound);
            game.motion_kags.insert(slot, kag);
            for op in ops {
                game.adv_ui(op, &mut opener)?;
            }
        }
        // ScriptManager.yotogi_kag.
        game.yotogi_kag.update(game.now, false, false, &game.files, storage.as_ref())?;
        if std::mem::take(&mut game.yotogi_kag.voice_wait) {
            // TagVoiceWait: SetWait(maid 0's AudioMan.GetLength()).
            let len = game.voice.as_ref().map_or(0, |v| v.2);
            game.yotogi_kag.set_wait(game.now, len, true);
        }
        game.sound_ops.append(&mut game.yotogi_kag.sound);
        for op in std::mem::take(&mut game.yotogi_kag.ui) {
            game.adv_ui(op, &mut opener)?;
        }
        match step {
            KagStep::Running | KagStep::Waiting => {}
            KagStep::SceneCall(name) => game.pending_scene = Some(name),
        }
        if let Some(name) = game.pending_scene.take() {
            game.load_scene(&mut opener, &name)?;
        }
        for (si, node, file) in std::mem::take(&mut game.pending_textures) {
            let bytes = game.files.read(storage.as_ref(), &file)?;
            let png = crysirror_image::kiss_tex::payload(&bytes)?;
            let img = crysirror_image::decode_standard(png)?;
            game.next_model += 1;
            let asset = host.upload_rgba(self.client, 0x7000_0000 + game.next_model, img.width, img.height, &img.rgba)?.relative().to_owned();
            if let Some(level) = game.scene_mut(si) {
                let tex = crate::ngui::Texture { asset, width: img.width as f32, height: img.height as f32 };
                let is_2d = level.scene.script(node, "UI2DSprite").is_some();
                let st = level.states.entry(node).or_default();
                if is_2d {
                    // Sprite.Create over the whole image; SetDimensions to the
                    // image unless the caller fixed the size (IconSize).
                    st.size.get_or_insert([tex.width, tex.height]);
                    st.sprite2d = Some(Some(tex));
                } else {
                    st.texture = Some(tex);
                }
            }
        }
        for (id, name, glb) in std::mem::take(&mut game.pending_models) {
            let asset = host.upload_model(self.client, 0x5000_0000 + id, &glb)?.relative().to_owned();
            // `bg:<id>` models belong to BgMgr; others are level content.
            let entity = match name.strip_prefix("bg:").and_then(|id| id.parse::<u32>().ok()) {
                Some(id) => id,
                None => {
                    game.next_entity += 1;
                    let entity = 0x100 + game.next_entity;
                    game.level_entities.push(entity);
                    entity
                }
            };
            let mut spec = json!({"id": entity, "name": name, "geometry": {"kind": "scene", "asset": asset}});
            if let Some(i) = game.pending_parents.iter().position(|p| p.0 == entity) {
                let (_, parent, _, transform) = game.pending_parents.remove(i);
                spec["parent"] = json!(parent);
                spec["transform"] = transform;
            }
            game.world.push(json!({"op": "spawn", "entity": spec}));
        }
        let mut ops = std::mem::take(&mut game.sound_ops);
        ops.append(&mut game.adv.sound);
        for op in ops {
            match op {
                crate::sound::SoundOp::Play { kind, file, fade_s, looping } => {
                    if kind == crate::sound::Kind::Voice {
                        let len = game.voice_length(storage.as_ref(), &file)?;
                        game.voice = Some((file.clone(), game.now, len, looping));
                    }
                    game.sound.play(host, self.client, &game.files, kind, &file, fade_s, looping)?
                }
                crate::sound::SoundOp::Volume { kind, value } => game.sound.set_volume(host, self.client, kind, value)?,
                crate::sound::SoundOp::Stop { kind, fade_s } => {
                    if kind == crate::sound::Kind::Voice {
                        game.voice = None;
                    }
                    game.sound.stop(host, self.client, kind, fade_s)?
                }
            }
        }
        game.sound.poll(host, self.client)?;
        // Maid.Update face step for each maid.
        let dt = game.now.saturating_sub(game.last_now) as f32 / 1000.;
        game.last_now = game.now;
        let t = game.now as f32 / 1000.;
        for c in game.characters.iter_mut().filter(|c| !c.man && c.visible) {
            let sets = std::mem::take(&mut c.blend_sets);
            c.morphs = c.face.update(dt, t, None, &sets);
            c.blend_sets = sets;
        }
        {
            let client = self.client;
            let mut upload = |id: u32, glb: &[u8]| -> Result<String> { Ok(host.upload_model(client, 0x6000_0000 + id, glb)?.relative().to_owned()) };
            let mut cmds = game.chara_view.sync(&mut game.characters, [game.all_pos[0] + game.all_offset[0], game.all_pos[1] + game.all_offset[1], game.all_pos[2] + game.all_offset[2]], game.all_rot, &game.files, storage.as_ref(), game.now, &mut upload, &game.res_textures)?;
            game.world.append(&mut cmds);
            for asset in std::mem::take(&mut game.chara_view.released) {
                host.release_model(client, &VfsPath::new(Mount::Asset, &asset)?)?;
            }
        }
        if game.level.as_ref().is_some_and(|l| l.name != "SceneTitle") {
            let camera = game.camera_command();
            game.world.push(camera);
        }
        let world = std::mem::take(&mut game.world);
        // Present: NGUI layers of the persistent root and the level, the fade.
        let mut uploader = HostUploader { host, client: self.client };
        let mut scenes_list: Vec<(&Scene, &HashMap<usize, NodeState>)> = vec![(&game.main.scene, &game.main.states)];
        if let Some(level) = &game.level {
            scenes_list.push((&level.scene, &level.states));
        }
        let mut layers = game.ui.compose(&mut game.unity, &mut opener, &mut uploader, &scenes_list)?;
        let fade = game.fade.value(game.now);
        layers.push(json!({"op": "set_opacity", "id": FADE_LAYER, "opacity": (fade.clamp(0., 1.) * 255. + 0.5) as u8}));
        let fade3d = game.fade3d.value(game.now);
        layers.push(json!({"op": "set_opacity", "id": FADE3D_LAYER, "opacity": (fade3d.clamp(0., 1.) * 255. + 0.5) as u8}));
        host.submit_engine(self.client, 1, &serde_json::to_vec(&json!({"layers": layers, "world": world}))?)?;
        if quit || game.quit_requested {
            // GameMain.ToApplicationQuit.
            return Ok(VmYield::Halted);
        }
        Ok(VmYield::WaitUntil(game.now + 16))
    }
}

impl VmAdapter for Cm3d2Vm {
    fn engine_id(&self) -> &'static str {
        ENGINE_ID
    }
    fn snapshot_schema(&self) -> u32 {
        1
    }
    fn preparation_pending(&self) -> bool {
        false
    }
    fn step(&mut self, host: &mut dyn HostServices, input: InputFrame, _budget: u32) -> crysirror_core::Result<VmYield> {
        match self.step_inner(host, input) {
            Ok(v) => Ok(v),
            Err(e) => {
                let message = format!("{e:#}");
                self.error = Some(message.clone());
                Err(Error::Io(message))
            }
        }
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported("CM3D2 framework snapshot"))
    }
    fn restore(&mut self, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported("CM3D2 framework snapshot"))
    }
    fn diagnostics(&self) -> Vec<u8> {
        let Some(g) = &self.game else { return Vec::new() };
        serde_json::to_vec(&json!({
            "scene": g.level.as_ref().map(|l| l.name.clone()),
            "active": format!("{:?}", g.active),
            "schedule": if g.level.as_ref().is_some_and(|level| level.name == "SceneDaily") {
                g.adv.tjs.lock().ok().map(|mut tjs| {
                    let state = &tjs.runtime.host_mut().state;
                    json!({"day": state.player.days, "slots": (0..crate::params::SCHEDULE_SLOTS).map(|slot| {
                        let maid = state.schedule_slot(slot)?;
                        let status = state.maids.get(&maid)?;
                        Some(json!({"maid":maid, "day_work":status.noon_work_id, "night_work":status.night_work_id,
                            "day_work_plays":status.work_data.get(&status.noon_work_id).map_or(0, |work| work.play_count)}))
                    }).collect::<Vec<_>>()})
                })
            } else { None },
            "kag": g.adv.position(),
            "kag_wait": g.adv.wait_state(),
            "yotogi": match &g.active { scenes::Active::Yotogi(y) => json!(y.diagnostics()), _ => json!(null) },
            "yotogi_kag": [g.yotogi_kag.position(), g.yotogi_kag.wait_state()],
            "yotogi_trace": g.yotogi_kag.trace,
            "camera": [g.camera.0, g.camera.1, g.camera.2],
            "characters": g.characters.iter().enumerate().filter(|(_, c)| c.visible).map(|(i, c)| json!([i, c.man, c.position])).collect::<Vec<_>>(),
            "all_pos": [g.all_pos, g.all_rot],
            "kag_trace": g.adv.trace,
            "hits": g.ui.hits.iter().filter_map(|h| {
                let scene = if h.scene == 0 { &g.main.scene } else { &g.level.as_ref()?.scene };
                Some(json!({"path": scene.path(h.node), "rect": [h.rect.x0, h.rect.y0, h.rect.x1, h.rect.y1]}))
            }).collect::<Vec<_>>(),
            "choices": g.choices.iter().map(|c| {
                let r = g.ui.rects.get(&(0, c.node)).or_else(|| g.main.scene.nodes[c.node].children.iter().find_map(|k| g.ui.rects.get(&(0, *k))));
                json!({"text": c.text, "rect": r.map(|r| [r.x0, r.y0, r.x1, r.y1])})
            }).collect::<Vec<_>>(),
            "fade": [g.fade.done(g.now), g.fade3d.done(g.now)],
            "error": self.error,
            "log": g.log.iter().rev().take(32).collect::<Vec<_>>(),
        }))
        .unwrap_or_default()
    }
}

#[cfg(feature = "runtime")]
pub fn register(registry: &mut crysirror_runtime::AdapterRegistry) -> Result<()> {
    registry.register("com3d2", Arc::new(crate::com3d2::Factory))?;
    registry.register(ENGINE_ID, Arc::new(Factory))
}
#[cfg(feature = "runtime")]
struct Factory;
#[cfg(feature = "runtime")]
impl crysirror_runtime::runtime::VmFactory for Factory {
    fn create(&self, client: ClientId, spec: &crysirror_runtime::model::ClientSpec) -> Result<Box<dyn VmAdapter>> {
        ensure!(spec.program.is_empty(), "CM3D2 executes native scenes and KAG, not data actions");
        Ok(Box::new(Cm3d2Vm::new(client, serde_json::from_value(spec.config.clone())?)?))
    }
}
