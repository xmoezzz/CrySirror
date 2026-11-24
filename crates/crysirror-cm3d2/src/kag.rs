// SPDX-License-Identifier: MPL-2.0
//! `ADVKagManager` over the plug-in's KAGParser (`DLL_KAG_*`).
//!
//! Native dialect (cm3d2_x64.dll, tag loop around 0x18003F9xx): `jump`,
//! `call` and `return` take `file`/`label` instead of KAG3's
//! `storage`/`target`, and a file name without extension gets `.ks`. Text
//! between tags accumulates until `GetText`/`TextClear`. `Exec` runs tags
//! until a registered callback returns true (the C# callbacks' "stop"), the
//! scenario ends, or `[s]` stops it. Tag names compare in lower case, as
//! `AddTagCallBack` registers them.
use crate::{files::GameFiles, tjs::Tjs};
use anyhow::{bail, Context, Result};
use crysirror_core::service::ClientStorage;
use crysirror_kag::{KagError, KagHost, KagParser, ParserOptions, Tag};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

/// `ScriptManager.tjs_`, shared by the ADV and motion KAG managers.
pub type SharedTjs = Arc<Mutex<Tjs>>;

/// Upper bound on tags run by one `Exec` before yielding to the frame.
const TAGS_PER_EXEC: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KagStep {
    Running,
    Waiting,
    /// `[SceneCall name=...]`: load this Unity scene.
    SceneCall(String),
}

/// Which `BaseKagManager` subclass a KAG runs as; each registers its own
/// tags, and the native KAG ignores tags without a registered callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KagKind {
    Adv,
    Motion,
    Yotogi,
}

/// Tags only `ADVKagManager.Initialize` registers.
const ADV_ONLY: &[&str] = &["scenecall", "talk", "hitret", "messagewindow", "fade", "choicesset", "choicesrandomset", "choicesrandomselect", "_choicesshow"];

/// Presentation requests of ADV tags, executed by the adapter.
#[derive(Clone, Debug, PartialEq)]
pub enum AdvUi {
    /// `MessageWindowMgr.OpenMessageWindowPanel` / `Close…`.
    Window(bool),
    /// `MessageWindowMgr` text and speaker (`[hitret]`).
    Message { name: String, text: String },
    ClearText,
    /// `CameraMain.FadeIn/FadeOut` (`to` = cover opacity), seconds.
    Fade { to: f32, seconds: f32 },
    /// `CameraMain.FadeInNoUI/FadeOutNoUI`.
    Fade3d { to: f32, seconds: f32 },
    /// `CameraMain.SetControl`.
    CameraControl(bool),
    /// `BgMgr.ChangeBg` (`Resources.Load("BG/" + name)`).
    SetBg(String),
    /// `CharacterMgr.CharaVisible(slot, visible, man)`.
    CharaVisible { man: bool, slot: usize, visible: bool },
    CharaVisibleAllOff { man: bool },
    /// `CharacterMgr.ActivateNpc` / `Activate(slot, stock)`.
    RetailNpc { man: bool, slot: usize, name: String, load: bool },
    CharaActivate { man: bool, slot: usize, npc: Option<String>, stock: Option<usize> },
    CharaDeactivate { man: bool, slot: usize },
    /// `MainCamera.SetTargetPos/SetDistance/SetAroundAngle`.
    Camera { target: [f32; 3], distance: f32, around: [f32; 2] },
    CameraReset,
    /// `CameraAnimationStart`: tween to the target state over `ms`.
    CameraAnimation { from: Option<([f32; 3], f32, [f32; 2])>, to: ([f32; 3], f32, [f32; 2]), delay_ms: u64, ms: u64, ease: String },
    /// `GameMain.MainLight` rotation (Euler) and intensity.
    LightMain { rotation: Option<[f32; 3]>, intensity: Option<f32> },
    /// `ScriptManager.LoadMotionScript(slot, false, file, label)`.
    RetailMotionScript { slot:i32, file:String, label:String, maid:Option<usize>, man:Option<usize>, positions:bool },
    MotionScript { slot: i32, file: String, label: String },
    /// `PlayMaidMotion(maid, file, loop, blend)`.
    Motion { man: bool, slot: usize, file: String, looping: bool, blend_s: f32, queue: bool },
    /// `CharacterMgr.SetCharaAllPos/SetCharaAllRot`.
    AllPos { position: Option<[f32; 3]>, rotation: Option<[f32; 3]> },
    /// `Maid.SetPos/SetRot` relative to the all-position.
    Offset { man: bool, slot: usize, position: Option<[f32; 3]>, rotation: Option<[f32; 3]> },
    /// `Maid.FaceAnime(name)`.
    Face { man: bool, slot: usize, name: String },
    /// `CharacterMgr.SetMaidOffsetPos` / `SetMaidOffsetOther` (`multi`).
    MaidOffset { list: Vec<(usize, CharaPer)>, split: i32, multi: bool },
    /// `CharacterMgr.SetManOffsetPos` (character offset only).
    ManOffset { man: usize, target: (bool, usize), split: i32, per: CharaPer },
    /// `ScriptManager.is_motion_blend`.
    MotionBlend(bool),
    /// `ScriptManager.StopMotionScript`.
    StopMotionScripts,
    RetailDeactivate(usize),
    RetailTrophy(i32),
    RetailClearMotionStacks,
    RetailSwap { man:bool, slot:usize, other:usize },
    RetailStock { index: usize, slot: usize, visible: Option<bool> },
    /// `YotogiManager.TagYotogiCall` (on the ADV KAG).
    YotogiCall { name: String, label: String, edit_label: Option<String> },
    /// `YotogiManager.TagPlayBGMRoom`: the selected stage's BGM.
    PlayBgmRoom { fade_s: f32 },
    /// `YotogiManager.TagControlMask`.
    ControlMask,
    /// `YotogiKagManager.TagTalkRepeat` → `YotogiPlayManager.SetRepeatVoiceFile`.
    RepeatVoice(String),
    /// `BaseKagManager.TagCharaActivateLeader`.
    CharaActivateLeader { slot: usize },
    /// `Maid.StopAnime`.
    MotionStop { man: bool, slot: usize },
    /// `TBody.StopAnimeLayer` for each layer.
    MotionLayerStop { man: bool, slot: usize, layers: Vec<i32> },
    /// `Maid.SetAutoTwist(kind, on)`.
    AutoTwist { man: bool, slot: usize, twists: Vec<(&'static str, bool)> },
    /// `Maid.SetModeHairR`.
    HairPhysics { man: bool, slot: usize, mode: String },
    /// `TBody.MuneYureL/R` (`None` keeps the side).
    BustMove { man: bool, slot: usize, left: Option<f32>, right: Option<f32> },
    /// `TBody.SetBoneHitHeightY`.
    PhysicsHit { man: bool, slot: usize, height: f32 },
    /// `Maid.IKTargetToAttachPoint` / `IKTargetToBone` (`target` `None`
    /// detaches).
    Ik { src: (bool, usize), bone: String, target: Option<((bool, usize), IkTarget)>, offset: [f32; 3] },
    /// `CharacterMgr.CharaAllOfsetPos/Rot`.
    AllOffset { position: Option<[f32; 3]>, rotation: Option<[f32; 3]> },
    /// `CharacterMgr.ResetCharaPosAll`.
    AllResetPos,
    /// `CameraMain.SetChaseCamera` / `RemoveChaseCameraAll`.
    CameraChase { man: bool, slot: usize, bone: String, offset: [f32; 3], once: bool, seconds: f32 },
    CameraChaseStop,
    /// `AudioSourceMgr.LoadPlay` of a character, or the dummy voice.
    Voice { who: Option<(bool, usize)>, file: String, fade_s: f32, looping: bool },
    VoiceStop { man: bool, slot: usize },
    /// `SoundMgr.StopAll`.
    StopSound,
    /// `Maid.ItemChangeTemp`.
    ItemChange { man: bool, slot: usize, category: String, name: String },
    /// `Maid.SetProp(category, item, 0, temp)`.
    ItemSet { man: bool, slot: usize, category: String, item: String, temp: bool },
    /// `Maid.ResetProp` of each MPN.
    ItemReset { man: bool, slot: usize, props: Vec<String> },
    /// `Maid.AllProcProp(Seq)`.
    AllProcProp { man: bool, slot: usize },
    /// `TBody.SetMask(slot, visible)`.
    ItemMask { man: bool, slot: usize, body_slot: String, visible: bool },
    /// `TBody.SetMaskMode`.
    ItemMaskMode { man: bool, slot: usize, mode: String },
    /// `TBody.SetChinkoVisible` (men only).
    Chinko { slot: usize, on: bool },
    /// `Maid.FaceBlend`.
    FaceBlend { man: bool, slot: usize, name: String },
    /// `Maid.OpenMouth`.
    OpenMouth { man: bool, slot: usize, open: bool },
    /// `Maid.Lipsync`.
    Lipsync { man: bool, slot: usize, on: bool },
    /// `Maid.StartKuchipakuPattern` / `StopKuchipakuPattern`.
    Kuchipaku { man: bool, slot: usize, pattern: Option<(f32, String)> },
    /// `Maid.EyeToPosition/EyeToTarget/EyeToCamera/EyeToReset`.
    Eye { man: bool, slot: usize, look: EyeLook, blend_s: f32 },
    /// `Maid.SetPosOffsetFromUdeScl{X,Y,Z}`.
    MaidOffsetCategory { man: bool, slot: usize, category: String, offset: [Option<f32>; 3] },
    /// `BgMgr.AddPrefabToBg` / `DelPrefabFromBg` / `DelPrefabFromBgAll`.
    /// `SceneSingleEffect.TagStartMvpEffect`.
    StartMvpEffect,
    AddPrefabBg { src: String, name: String, dest: String, position: [f32; 3], rotation: [f32; 3] },
    DelPrefabBg(Option<String>),
    /// `BgMgr.MovePrefabToChara`.
    MovePrefabToChara { man: bool, slot: usize, name: String, bone: String, offset: [f32; 3], seconds: f32 },
    /// `Maid.AddPrefab` / `DelPrefab` / `DelPrefabAll`.
    AddPrefabChara { man: bool, slot: usize, src: String, name: String, bone: String, position: [f32; 3], rotation: [f32; 3] },
    DelPrefabChara { man: bool, slot: usize, name: Option<String> },
    /// `Maid.AnimationObject`.
    AnimObjChara { man: bool, slot: usize, name: String, anim: String, play: bool, looping: bool },
    /// `TBody.MulTexSet(.., add, x, y, r, s)` on the property (and
    /// `_ShadowTex` for `_MainTex`), `MulTexProc`.
    TexMulAdd { man: bool, slot: usize, body_slot: String, material: usize, property: String, layer: i32, file: String, at: (i32, i32, f32, f32) },
    /// `TBody.MulTexRemove` of layers 10..=20; the character becomes visible.
    TexMulDel { man: bool, slot: usize, body_slot: String, material: usize, property: String },
    /// `GameMain.OnStartDay` / `OnEndDay`.
    StartDay,
    EndDay,
    /// `MessageWindowMgr.CreateSelectButtons(choices)`: `(text, label)`.
    Choices(Vec<(String, String)>),
    RetailChoices(Vec<(String,String,bool)>),
    /// A delayed `SoundMgr` call (`playse wait=`).
    Sound(crate::sound::SoundOp),
    /// `TagHitRet` voice: `VoiceStopAll`, then the target maid's
    /// `AudioMan.LoadPlay` (or `PlayDummyVoice`) unless skipping. The maid
    /// is `maid=`, else found by `real=`/`name=` chara name.
    TalkVoice { file: String, maid: Option<(bool, usize)>, chara_name: Option<String> },
}

/// Target of an IK attachment.
#[derive(Clone, Debug, PartialEq)]
pub enum IkTarget {
    /// `targetobj` slot and `targetpoint` attach point.
    AttachPoint { slot: String, point: String },
    Bone(String),
}

/// `Maid.EyeTo*` targets.
#[derive(Clone, Debug, PartialEq)]
pub enum EyeLook {
    /// `TBody.trsLookTarget` unset and no tracking (the struct default).
    None,
    Position([f32; 3]),
    Chara { who: (bool, usize), bone: Option<String> },
    /// `Maid.EyeMoveType` name.
    Camera(String),
    Reset,
}

/// `CharacterMgr.CharaPer`: offsets at maximum/minimum DouPer and sintyou.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CharaPer {
    pub dou_greater: [f32; 3],
    pub dou_less: [f32; 3],
    pub shin_greater: [f32; 3],
    pub shin_less: [f32; 3],
    pub harem_dou_greater: [f32; 3],
    pub harem_dou_less: [f32; 3],
    pub harem_shin_greater: [f32; 3],
    pub harem_shin_less: [f32; 3],
}

impl Default for EyeLook {
    fn default() -> Self {
        Self::None
    }
}

/// `GetMaidAndMan`: `maid=n` or `man=n` (default maid 0).
fn who(a: &BTreeMap<String, String>) -> (bool, usize) {
    if let Some(m) = a.get("man") {
        (true, crate::text::atoi(m.as_bytes()).max(0) as usize)
    } else {
        (false, a.get("maid").map(|m| crate::text::atoi(m.as_bytes()).max(0) as usize).unwrap_or(0))
    }
}

/// `GetMaidAndMan(string)`: `maid:n` / `man:n`.
fn who_str(v: &str) -> Result<(bool, usize)> {
    let v = v.to_ascii_lowercase();
    let (kind, n) = v.split_once(':').unwrap_or((v.as_str(), "0"));
    let n = n.trim().parse::<usize>().with_context(|| format!("{v}: メイド,もしくは男の指定が不正です"))?;
    match kind {
        "maid" => Ok((false, n)),
        "man" => Ok((true, n)),
        _ => bail!("{v}: メイド,もしくは男の指定が不正です"),
    }
}

fn real(a: &BTreeMap<String, String>, k: &str) -> Option<f32> {
    a.get(k).map(|v| crate::text::atof(v.as_bytes()))
}

fn vec3(a: &BTreeMap<String, String>, keys: [&str; 3]) -> Option<[f32; 3]> {
    if keys.iter().any(|k| a.contains_key(*k)) {
        Some(keys.map(|k| real(a, k).unwrap_or(0.)))
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug)]
struct Wait {
    until: u64,
    skip: bool,
}

struct Ctx<'a> {
    adv: bool,
    r_depth: usize,
    files: &'a GameFiles,
    storage: &'a dyn ClientStorage,
    tjs: std::sync::MutexGuard<'a, Tjs>,
}

fn scenario_name(storage: &str) -> String {
    if storage.contains('.') {
        storage.to_owned()
    } else {
        format!("{storage}.ks")
    }
}

/// `ScriptManager.ReplacePersonal`: `?` becomes the `PersonalChar` (C, B,
/// A for Pure, Cool, Pride) of maid 0.
fn replace_personal(tjs: &Tjs, name: &str) -> String {
    if !name.contains('?') {
        return name.to_owned();
    }
    let personal = tjs.runtime.host().state.maid(0).map_or(0, |m| m.personal);
    let c = ["C", "B", "A"].get(personal as usize).copied().unwrap_or("");
    name.replace('?', c)
}

impl Ctx<'_> {
    fn check_target(&self, storage:Option<&str>)->crysirror_kag::Result<bool>{
        let state=&self.tjs.runtime.host().state.retail;
        if state.compatibility && let Some(file)=storage.filter(|f|!f.is_empty()) {
            let name=scenario_name(&replace_personal(&self.tjs,file));
            if !state.files(self.files).exists(&name){return Err(KagError::host(format!("missing legacy script {name}")));}
        }
        Ok(true)
    }
}

impl KagHost for Ctx<'_> {
    fn on_jump(&mut self,_:&Tag,storage:Option<&str>,_:Option<&str>)->crysirror_kag::Result<bool>{ self.check_target(storage) }
    fn on_call(&mut self,_:&Tag,storage:Option<&str>,_:Option<&str>)->crysirror_kag::Result<bool>{ self.check_target(storage) }
    fn on_return(&mut self,_:&Tag,storage:Option<&str>,_:Option<&str>)->crysirror_kag::Result<bool>{ self.check_target(storage) }
    fn sync_parser_state(&mut self, parser:&KagParser)->crysirror_kag::Result<()>{
        if self.adv { self.tjs.runtime.host_mut().state.retail.adv_depth=(parser.call_stack_depth(),self.r_depth); }
        Ok(())
    }
    fn on_call_stack_depth(&mut self, depth:usize)->crysirror_kag::Result<()>{
        if self.adv && let Some(caller)=depth.checked_sub(1) {
            self.tjs.runtime.host_mut().state.retail.returned_to((caller,self.r_depth));
        }
        Ok(())
    }
    fn load_scenario(&mut self, storage: &str) -> crysirror_kag::Result<String> {
        // KagScript.SetReplaceEvent(ReplaceFileNameCallBack).
        let name = scenario_name(&replace_personal(&self.tjs, storage));
        let bytes = self.tjs.runtime.host().state.retail.files(self.files).read(self.storage, &name).map_err(|e| KagError::host(format!("{e:#}")))?;
        Ok(encoding_rs::SHIFT_JIS.decode(&bytes).0.into_owned())
    }
    fn eval_bool(&mut self, expression: &str) -> crysirror_kag::Result<bool> {
        self.tjs.eval_bool(expression).map_err(|e| KagError::host(format!("{e:#}")))
    }
    fn eval_string(&mut self, expression: &str) -> crysirror_kag::Result<String> {
        self.tjs.eval_string(expression).map_err(|e| KagError::host(format!("{e:#}")))
    }
}

pub struct AdvKag {
    parser: KagParser,
    pub tjs: SharedTjs,
    pub enabled: bool,
    loaded: bool,
    exec_requested: bool,
    stopped: bool,
    wait: Option<Wait>,
    /// Executed tags (diagnostics, with `CM3D2_TRACE` set).
    pub trace: Option<Vec<String>>,
    pub text: String,
    /// `ADVKagManager.tag_backup`: the attributes of the last `SceneCall`.
    pub tag_backup: BTreeMap<String, String>,
    /// `ScriptManager.is_motion_blend`.
    pub motion_blend: bool,
    /// `SoundMgr` calls of this frame's tags.
    pub sound: Vec<crate::sound::SoundOp>,
    // Preserve the filesystem mode at each retail sound tag, including switches within one Exec.
    pub(crate) retail_sound: Option<Vec<(crate::sound::SoundOp,bool)>>,
    pub ui: Vec<AdvUi>,
    /// `BaseKagManager.fade_wait_`.
    fade_wait: bool,
    /// Waiting for the message window's click after `[hitret]`.
    hitret_wait: bool,
    /// Time the `[hitret]` text was shown (`MessageClass.exit_wait_count_`).
    hitret_at: u64,
    talk_name: String,
    /// `TagTalk` voice: file and speaking character.
    talk_voice: Option<(String, Option<(bool, usize)>)>,
    talk_real: Option<String>,
    /// A scenario to load after the current tag (`R_call`/`R_return`).
    pending_load: Option<(String, Option<String>)>,
    /// `BaseKagManager.wait_event_list_`: name → `(due ms, delay ms, op)`.
    wait_events: BTreeMap<&'static str, Vec<(u64, u64, AdvUi)>>,
    /// `choise_data_` / `choise_random_data_`.
    choices: Vec<(String, String,bool)>,
    random_choices: Vec<(String, String,bool)>,
    /// `KagStackCall` of `R_call`: caller storage, called stem, id.
    r_stack: Vec<(String, String, i32)>,
    /// xorshift state for `Random` draws.
    rng: u64,
    /// `MotionKagManager.next_flag`.
    pub next_motion: bool,
    pub kind: KagKind,
    /// `YotogiKagManager.TagVoiceWait` asked for maid 0's voice length.
    pub voice_wait: bool,
}

fn attrs(tag: &Tag) -> BTreeMap<String, String> {
    tag.attributes
        .iter()
        .filter_map(|a| match a {
            crysirror_kag::Attribute::Named { name, value } => Some((name.to_ascii_lowercase(), value.raw().to_owned())),
            crysirror_kag::Attribute::Spread => None,
        })
        .collect()
}

fn property<'a>(a: &'a BTreeMap<String, String>, tag: &str, name: &str) -> Result<&'a str> {
    a.get(name).map(String::as_str).with_context(|| format!("[{tag}] requires {name}"))
}

/// `KagTagSupport.GetTagProperty(..).AsBool()`.
fn as_bool(v: &str) -> bool {
    let v = v.trim();
    v.eq_ignore_ascii_case("true") || crate::text::atoi(v.as_bytes()) != 0
}

impl AdvKag {
    pub fn new(files: &GameFiles, storage: &dyn ClientStorage) -> Result<Self> {
        Self::for_profile(files, storage, crate::tjs::ScriptProfile::Cm3d2, Default::default())
    }

    pub(crate) fn for_profile(files: &GameFiles, storage: &dyn ClientStorage, profile: crate::tjs::ScriptProfile, state:crate::tjs::TjsState) -> Result<Self> {
        let mut sources = HashMap::new();
        for name in ["startup.tjs", "utils.tjs", "game_utils.tjs"] {
            if files.exists(name) {
                let bytes = files.read(storage, name)?;
                sources.insert(name.to_owned(), encoding_rs::SHIFT_JIS.decode(&bytes).0.into_owned());
            }
        }
        let options = ParserOptions { storage_attribute: "file".into(), target_attribute: "label".into(), ..Default::default() };
        let mut parser = KagParser::with_options(options);
        // ADVKagManager.Initialize: LoadScenarioString of the macro.
        parser.set_macro_definitions([("choicesshow", "\t@_choicesshow\n\t@s\n")]);
        Ok(Self {
            parser,
            tjs: Arc::new(Mutex::new(Tjs::with_state(sources, profile, state)?)),
            enabled: false,
            loaded: false,
            exec_requested: false,
            stopped: false,
            wait: None,
            text: String::new(),
            tag_backup: BTreeMap::new(),
            motion_blend: true,
            sound: Vec::new(),
            retail_sound: (profile == crate::tjs::ScriptProfile::Com3d2).then(Vec::new),
            ui: Vec::new(),
            fade_wait: false,
            hitret_wait: false,
            hitret_at: 0,
            talk_name: String::new(),
            talk_voice: None,
            talk_real: None,
            pending_load: None,
            wait_events: BTreeMap::new(),
            choices: Vec::new(),
            random_choices: Vec::new(),
            r_stack: Vec::new(),
            trace: std::env::var_os("CM3D2_TRACE").map(|_| Vec::new()),
            rng: 0x2545_f491_4f6c_dd1d,
            next_motion: false,
            kind: KagKind::Adv,
            voice_wait: false,
        })
    }

    /// `PerthOffsetPosTagData.TagArrayData`: a TJS array attribute of 6
    /// numbers (greater xyz, less xyz).
    fn vec_pair(&self, a: &BTreeMap<String, String>, key: &str) -> Result<Option<([f32; 3], [f32; 3])>> {
        let Some(text) = a.get(&key.to_ascii_lowercase()) else { return Ok(None) };
        let mut tjs = self.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let v = tjs.eval(text)?;
        let crysirror_tjs2::runtime::Variant::Object(h) = v else { anyhow::bail!("{key} is not an array") };
        let items: Vec<f32> = tjs
            .runtime
            .array_elements(h)
            .unwrap_or(&[])
            .iter()
            .map(|e| match e {
                crysirror_tjs2::runtime::Variant::Integer(n) => *n as f32,
                crysirror_tjs2::runtime::Variant::Real(r) => *r as f32,
                _ => 0.,
            })
            .collect();
        anyhow::ensure!(items.len() == 6, "{key}: expected 6 elements, found {}", items.len());
        Ok(Some(([items[0], items[1], items[2]], [items[3], items[4], items[5]])))
    }

    /// `PerthOffsetPosTagData.GetCharaPer` with a per-maid suffix.
    fn chara_per(&self, a: &BTreeMap<String, String>, suffix: &str) -> Result<Option<CharaPer>> {
        let mut per = CharaPer::default();
        let mut any = false;
        if let Some((g, l)) = self.vec_pair(a, &format!("OffsDouPar{suffix}"))? {
            (per.dou_greater, per.dou_less, any) = (g, l, true);
        }
        if let Some((g, l)) = self.vec_pair(a, &format!("OffsShinCho{suffix}"))? {
            (per.shin_greater, per.shin_less, any) = (g, l, true);
        }
        if let Some((g, l)) = self.vec_pair(a, &format!("HaremDouPer{suffix}"))? {
            (per.harem_dou_greater, per.harem_dou_less, any) = (g, l, true);
        }
        if let Some((g, l)) = self.vec_pair(a, &format!("HaremShinCho{suffix}"))? {
            (per.harem_shin_greater, per.harem_shin_less, any) = (g, l, true);
        }
        Ok(any.then_some(per))
    }

    /// A `MotionKagManager` sharing the ADV manager's TJS context.
    pub fn motion(tjs: SharedTjs) -> Self {
        let options = ParserOptions { storage_attribute: "file".into(), target_attribute: "label".into(), ..Default::default() };
        Self {
            parser: KagParser::with_options(options),
            tjs,
            enabled: true,
            loaded: false,
            exec_requested: false,
            stopped: false,
            wait: None,
            text: String::new(),
            tag_backup: BTreeMap::new(),
            motion_blend: true,
            sound: Vec::new(),
            retail_sound: None,
            ui: Vec::new(),
            fade_wait: false,
            hitret_wait: false,
            hitret_at: 0,
            talk_name: String::new(),
            talk_voice: None,
            talk_real: None,
            pending_load: None,
            wait_events: BTreeMap::new(),
            choices: Vec::new(),
            random_choices: Vec::new(),
            r_stack: Vec::new(),
            trace: None,
            rng: 0x2545_f491_4f6c_dd1d,
            next_motion: false,
            kind: KagKind::Motion,
            voice_wait: false,
        }
    }

    /// `ScriptManager.yotogi_kag` (`YotogiKagManager`), disabled until
    /// `LoadYotogiScript`.
    pub fn yotogi(tjs: SharedTjs) -> Self {
        let mut k = Self::motion(tjs);
        k.kind = KagKind::Yotogi;
        k.enabled = false;
        k.trace = std::env::var_os("CM3D2_TRACE").map(|_| Vec::new());
        k
    }

    /// `BaseKagManager.SetWait(ms, skip_possible)`.
    pub fn set_wait(&mut self, now: u64, ms: u64, skip: bool) {
        self.wait = Some(Wait { until: now + ms, skip });
    }

    /// `kag.GetCurrentLabel()`.
    pub fn current_label(&self) -> &str {
        self.parser.cur_label().unwrap_or("")
    }

    /// `ScriptManager.LoadAdvScenarioScript` + `Exec`.
    pub fn load(&mut self, files: &GameFiles, storage: &dyn ClientStorage, file: &str, label: &str) -> Result<()> {
        self.enabled = true;
        let tjs = self.tjs.clone();
        let mut ctx = Ctx { adv: self.kind == KagKind::Adv, r_depth: self.r_stack.len(), files, storage, tjs: tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock poisoned"))? };
        self.parser.load_scenario_with(scenario_name(file), &mut ctx)?;
        if !label.is_empty() {
            self.parser.go_to_with(None, Some(label), &mut ctx)?;
        }
        self.loaded = true;
        self.stopped = false;
        Ok(())
    }

    /// `BaseKagManager.JumpLabel`.
    pub fn jump_label(&mut self, files: &GameFiles, storage: &dyn ClientStorage, label: &str) -> Result<()> {
        let tjs = self.tjs.clone();
        let mut ctx = Ctx { adv: self.kind == KagKind::Adv, r_depth: self.r_stack.len(), files, storage, tjs: tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock poisoned"))? };
        self.parser.go_to_with(None, Some(label), &mut ctx)?;
        self.stopped = false;
        Ok(())
    }

    /// `KagScript.Serialize` (`DLL_KAG_CreateSerializeData` → vtable slot 7,
    /// cm3d2_x64.dll `0x1800266d0`): `BasicKagScript` part [u32 size, the
    /// read-history block (`"reda"`, file count, per file a u64 name hash, a
    /// label count and u64 label hashes), u32 byte length, the UTF-16
    /// `Dictionary.toStructString` of `KAGParser.store()` with its NUL], the
    /// tag-callback flag (always 0 between tags) and `KagStackCall` (u32
    /// count; per frame UTF-16 caller storage and called stem with byte
    /// lengths including the NUL, i32 id). Read history is not kept by this
    /// port; an empty block is written (SATD-cm3d2-game-logic).
    pub fn serialize_blob(&self) -> Result<Vec<u8>> {
        let text = {
            let mut tjs = self.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let rt = &mut tjs.runtime;
            let v = crysirror_kag::krkr_store::capture(rt, &self.parser, crysirror_kag::krkr_store::Profile::Cm3d2).map_err(|e| anyhow::anyhow!("{e}"))?;
            crate::tjs::to_struct_string(rt, &v)
        };
        let mut out = Vec::new();
        let utf16 = |s: &str| -> Vec<u8> { s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect() };
        let history: Vec<u8> = [b"reda".as_slice(), &0u32.to_le_bytes()].concat();
        out.extend_from_slice(&(history.len() as u32).to_le_bytes());
        out.extend_from_slice(&history);
        let s = utf16(&text);
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(&s);
        out.push(0);
        out.extend_from_slice(&(self.r_stack.len() as u32).to_le_bytes());
        for (caller, stem, id) in &self.r_stack {
            for t in [caller, stem] {
                let b = utf16(t);
                out.extend_from_slice(&(b.len() as u32).to_le_bytes());
                out.extend_from_slice(&b);
            }
            out.extend_from_slice(&id.to_le_bytes());
        }
        Ok(out)
    }

    /// `KagScript.Deserialize`: restore the parser from the struct string,
    /// then `GoToLabel(GetCurrentLabel())`; the `R_call` stack follows.
    pub fn deserialize_blob(&mut self, files: &GameFiles, storage: &dyn ClientStorage, blob: &[u8]) -> Result<()> {
        let mut r = crate::binio::Reader::new(blob);
        let n = r.u32()? as usize;
        let history = r.take(n)?;
        anyhow::ensure!(history.len() >= 8 && &history[..4] == b"reda", "KAG read history header");
        let utf16 = |b: &[u8]| -> Result<String> {
            let units: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|u| *u != 0).collect();
            String::from_utf16(&units).context("KAG save UTF-16")
        };
        let n = r.u32()? as usize;
        let text = utf16(r.take(n)?)?;
        let _flag = r.take(1)?;
        let frames = r.u32()? as usize;
        anyhow::ensure!(frames <= 4096, "KagStackCall count {frames}");
        let mut r_stack = Vec::with_capacity(frames);
        for _ in 0..frames {
            let n = r.u32()? as usize;
            let caller = utf16(r.take(n)?)?;
            let n = r.u32()? as usize;
            let stem = utf16(r.take(n)?)?;
            r_stack.push((caller, stem, r.i32()?));
        }
        let tjs = self.tjs.clone();
        let mut ctx = Ctx { adv: self.kind == KagKind::Adv, r_depth: self.r_stack.len(), files, storage, tjs: tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock poisoned"))? };
        let dict = if text.is_empty() { None } else { Some(ctx.tjs.eval(&text)?) };
        if let Some(v) = dict {
            use crysirror_kag::krkr_store::{self, Profile};
            let h = v.object_handle().context("KAGParser save data is not a dictionary")?;
            if let Some(what) = krkr_store::unrestorable(&ctx.tjs.runtime, h, Profile::Cm3d2).map_err(|e| anyhow::anyhow!("{e}"))? {
                bail!("CM3D2 save: KAGParser state needs {what}");
            }
            let main = ctx.tjs.runtime.object_member(h, "storageName").to_tjs_string().map_err(|e| anyhow::anyhow!("{e}"))?;
            let mut storages = krkr_store::frame_storages(&ctx.tjs.runtime, h).map_err(|e| anyhow::anyhow!("{e}"))?;
            storages.push(main.clone());
            for s in storages {
                self.parser.load_scenario_with(s.clone(), &mut ctx).with_context(|| format!("load {s}"))?;
            }
            let snapshot = krkr_store::decode(&ctx.tjs.runtime, &self.parser, h, Profile::Cm3d2).map_err(|e| anyhow::anyhow!("{e}"))?;
            self.parser.restore(snapshot)?;
        }
        drop(ctx);
        self.r_stack = r_stack;
        self.loaded = true;
        self.stopped = false;
        self.wait = None;
        self.hitret_wait = false;
        self.wait_events.clear();
        self.choices.clear();
        self.text.clear();
        Ok(())
    }

    /// `BaseKagManager.fade_wait_` (saved with the KAG).
    pub fn fade_wait(&self) -> bool {
        self.fade_wait
    }

    pub fn set_fade_wait(&mut self, on: bool) {
        self.fade_wait = on;
    }

    /// `KagScript.ClearCallStack` (`DLL_KAG_ClearCallStack`). Whether the
    /// native `KagStackCall` list of `R_call` is part of that stack is not
    /// traced (SATD-cm3d2-game-logic); it is kept.
    pub fn clear_call_stack(&mut self) {
        self.parser.clear_call_stack();
    }

    pub fn exec(&mut self) {
        self.exec_requested = true;
    }

    /// The waits holding this KAG (diagnostics).
    pub fn wait_state(&self) -> String {
        format!(
            "enabled={} loaded={} stopped={} exec={} fade_wait={} hitret={} wait={:?} events={:?} calls={:?}",
            self.enabled,
            self.loaded,
            self.stopped,
            self.exec_requested,
            self.fade_wait,
            self.hitret_wait,
            self.wait.map(|w| w.until),
            self.wait_events.iter().map(|(k, v)| (k.to_string(), v.len())).collect::<Vec<_>>(),
            self.parser.call_stack().iter().map(|f| f.storage().to_owned()).collect::<Vec<_>>()
        )
    }

    pub fn position(&self) -> String {
        format!(
            "{}:{} {}",
            self.parser.cur_storage().unwrap_or(""),
            self.parser.cur_line().unwrap_or(0),
            self.parser.cur_label().unwrap_or("")
        )
    }

    /// `WaitEventList.Add`: run `op` after `delay` ms.
    fn add_event(&mut self, list: &'static str, now: u64, delay: u64, op: AdvUi) {
        self.wait_events.entry(list).or_default().push((now + delay, delay, op));
    }

    /// `WaitEventList.Clear`.
    fn clear_events(&mut self, list: &str) {
        self.wait_events.remove(list);
    }

    /// The delay of the last pending event of a list (`motionnext`).
    fn last_delay(&self, list: &str) -> Option<u64> {
        self.wait_events.get(list).and_then(|l| l.last()).map(|e| e.1)
    }

    /// `MotionKagManager.ClearMotion`: waits and wait events.
    pub fn clear_motion(&mut self) {
        self.wait = None;
        self.wait_events.clear();
    }

    /// `UnityEngine.Random.Range(lo, hi)` (integers, `hi` exclusive).
    fn random(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        lo + (x % (hi - lo) as u64) as i32
    }

    /// The select-button callback of `Tag_ChoicesShow`: go to the label
    /// and execute.
    pub fn choose(&mut self, files: &GameFiles, storage: &dyn ClientStorage, label: &str) -> Result<()> {
        self.jump_label(files, storage, label)?;
        self.exec();
        Ok(())
    }

    /// `BaseKagManager.Update` for this frame.
    /// When the text of the pending `[hitret]` was shown.
    pub fn hitret_since(&self) -> Option<u64> {
        self.hitret_wait.then_some(self.hitret_at)
    }

    pub fn update(&mut self, now: u64, click: bool, fading: bool, files: &GameFiles, storage: &dyn ClientStorage) -> Result<KagStep> {
        if !self.enabled || !self.loaded {
            return Ok(KagStep::Waiting);
        }
        // WaitEventList.EventCheck of every list.
        for list in self.wait_events.values_mut() {
            let (due, rest): (Vec<_>, Vec<_>) = std::mem::take(list).into_iter().partition(|e| e.0 <= now);
            *list = rest;
            self.ui.extend(due.into_iter().map(|e| e.2));
        }
        if self.fade_wait {
            if fading {
                return Ok(KagStep::Waiting);
            }
            self.fade_wait = false;
            self.exec_requested = true;
        }
        if self.hitret_wait {
            if !click {
                return Ok(KagStep::Waiting);
            }
            // MessageWindowMgr click: clear and continue.
            self.hitret_wait = false;
            self.exec_requested = true;
        }
        if let Some(w) = self.wait {
            if now < w.until && !(w.skip && click) {
                return Ok(KagStep::Waiting);
            }
            self.wait = None;
            self.exec_requested = true;
        }
        if !self.exec_requested {
            return Ok(KagStep::Waiting);
        }
        self.exec_requested = false;
        self.run(now, files, storage)
    }

    fn run(&mut self, now: u64, files: &GameFiles, storage: &dyn ClientStorage) -> Result<KagStep> {
        for _ in 0..TAGS_PER_EXEC {
            let tjs = self.tjs.clone();
            let mut ctx = Ctx { adv: self.kind == KagKind::Adv, r_depth: self.r_stack.len(), files, storage, tjs: tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock poisoned"))? };
            let next = self.parser.next_tag_with(&mut ctx);
            if ctx.adv {
                ctx.tjs.runtime.host_mut().state.retail.adv_depth=(self.parser.call_stack_depth(), self.r_stack.len());
            }
            if std::mem::take(&mut ctx.tjs.runtime.host_mut().state.retail.stop_motions) {
                self.ui.push(AdvUi::StopMotionScripts);
            }
            if self.retail_sound.is_some() {
                use crate::tjs::Request;
                let state=&mut ctx.tjs.runtime.host_mut().state;
                for request in std::mem::take(&mut state.requests) {
                    match request {
                        Request::DeactivateMaid(slot) => self.ui.push(AdvUi::RetailDeactivate(slot)),
                        Request::RefreshMaid(slot) => self.ui.push(AdvUi::AllProcProp {man:false,slot}),
                        Request::CreateMaid(index) => self.ui.push(AdvUi::RetailStock {index, slot:0, visible:Some(true)}),
                        Request::SetActiveMaid {slot,chara,visible} => self.ui.push(AdvUi::RetailStock {index:chara,slot,visible:Some(visible)}),
                        Request::ClearCallStack => {self.parser.clear_call_stack(); self.ui.push(AdvUi::RetailClearMotionStacks);},
                        Request::TrophyEffect(id) => self.ui.push(AdvUi::RetailTrophy(id)),
                    }
                }
            }
            drop(ctx);
            let Some(tag) = next.with_context(|| self.position())? else {
                self.stopped = true;
                return Ok(KagStep::Waiting);
            };
            let name = tag.tagname.to_ascii_lowercase();
            let a = attrs(&tag);
            if self.trace.is_some() {
                let line = format!("{} {name} {:?}", self.position(), a);
                let t = self.trace.as_mut().unwrap();
                t.push(line);
                if t.len() > 400 {
                    t.remove(0);
                }
            }
            match name.as_str() {
                "ch" => {
                    self.text.push_str(a.get("text").map(String::as_str).unwrap_or(""));
                    continue;
                }
                "r" | "interrupt" => continue,
                // Unregistered name tags stay in the text for
                // ScriptManager.ReplaceCharaName.
                "sf" | "hf" | "hl" => {
                    self.text.push_str(&format!("[{}]", tag.tagname));
                    continue;
                }
                "hlf" | "hf1" | "hf2" | "hf3" | "hf4"
                | "hl1" | "hl2" | "hl3" | "hl4"
                | "hlf1" | "hlf2" | "hlf3" | "hlf4" if self.retail_sound.is_some() => {
                    self.text.push_str(&format!("[{}]", tag.tagname));
                    continue;
                }
                "s" => {
                    self.stopped = true;
                    return Ok(KagStep::Waiting);
                }
                _ => {}
            }
            let step=self.tag(now, &name, &a).with_context(|| format!("[{name}] at {}", self.position()))?;
            if let Some(queue)=self.retail_sound.as_mut() && !self.sound.is_empty() {
                let mode=tjs.lock().map_err(|_|anyhow::anyhow!("TJS lock"))?.runtime.host().state.retail.compatibility;
                queue.extend(self.sound.drain(..).map(|op|(op,mode)));
            }
            if let Some(step) = step {
                return Ok(step);
            }
            if let Some((file, label)) = self.pending_load.take() {
                let tjs = self.tjs.clone();
                let mut ctx = Ctx { adv: self.kind == KagKind::Adv, r_depth: self.r_stack.len(), files, storage, tjs: tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock poisoned"))? };
                self.parser.load_scenario_with(file.clone(), &mut ctx).with_context(|| format!("load {file}"))?;
                if let Some(label) = label {
                    self.parser.go_to_with(None, Some(&label), &mut ctx).with_context(|| format!("{file} {label}"))?;
                }
            }
        }
        // Yield; continue on the next frame.
        self.exec_requested = true;
        Ok(KagStep::Running)
    }

    /// One registered tag callback. `Some` stops `Exec`.
    fn tag(&mut self, now: u64, name: &str, a: &BTreeMap<String, String>) -> Result<Option<KagStep>> {
        if self.kind == KagKind::Yotogi {
            if let Some(step) = self.yotogi_tag(now, name, a)? {
                return Ok(step);
            }
        }
        if self.kind != KagKind::Adv && ADV_ONLY.contains(&name) {
            return Ok(None);
        }
        Ok(match name {
            // ADVKagManager.TagSceneCall.
            "scenecall" => {
                let scene = property(a, name, "name")?.to_owned();
                self.tag_backup = a.clone();
                self.ui.push(AdvUi::StopMotionScripts);
                // MessageWindowMgr.CloseMessageWindowPanel + CancelSkipAndAuto.
                self.ui.push(AdvUi::Window(false));
                self.motion_blend = true;
                self.ui.push(AdvUi::MotionBlend(true));
                if let Some(e) = a.get("enabled") {
                    self.enabled = as_bool(e);
                }
                Some(KagStep::SceneCall(scene))
            }
            // BaseKagManager.TagWait.
            "wait" => {
                self.wait = None;
                let time = crate::text::atoi(property(a, name, "time")?.as_bytes());
                if time <= 0 {
                    None
                } else {
                    let skip = a.get("skip").is_none_or(|v| as_bool(v));
                    self.wait = Some(Wait { until: now + time as u64, skip });
                    Some(KagStep::Waiting)
                }
            }
            // ScriptManager.TagMotionBlendEnabled sets is_motion_blend.
            "motionblendenabled" => {
                self.motion_blend = !a.contains_key("false");
                self.ui.push(AdvUi::MotionBlend(self.motion_blend));
                None
            }
            // Original no-ops (TagPosition/TagRotation/TagTransform only look
            // the character up).
            "position" | "rotation" | "transform" => None,
            "setmaidoffsetpos" | "setmaidoffsetmultipos" => {
                let split = crate::text::atoi(property(a, name, "splitnum")?.as_bytes());
                anyhow::ensure!(split != 0, "splitnum must not be 0");
                let mut list = Vec::new();
                if name == "setmaidoffsetpos" {
                    let (_, slot) = who(a);
                    list.push((slot, self.chara_per(a, "")?.unwrap_or_default()));
                } else {
                    for i in 0..18 {
                        if let Some(per) = self.chara_per(a, &format!("-maid:{i}"))? {
                            list.push((i, per));
                        }
                    }
                }
                self.ui.push(AdvUi::MaidOffset { list, split, multi: name == "setmaidoffsetmultipos" });
                None
            }
            "setmanoffsetpos" => {
                let (_, man) = who(a);
                let split = crate::text::atoi(property(a, name, "splitnum")?.as_bytes());
                anyhow::ensure!(split != 0, "splitnum must not be 0");
                let target = property(a, name, "target")?.to_ascii_lowercase();
                let target = match target.split_once('=') {
                    Some(("man", n)) => (true, crate::text::atoi(n.as_bytes()).max(0) as usize),
                    Some((_, n)) => (false, crate::text::atoi(n.as_bytes()).max(0) as usize),
                    None => (false, crate::text::atoi(target.trim_start_matches(|c: char| !c.is_ascii_digit()).as_bytes()).max(0) as usize),
                };
                let per = self.chara_per(a, "")?.unwrap_or_default();
                // SATD-cm3d2-game-logic: PenisPer (vecChinkoOffset) placement.
                self.ui.push(AdvUi::ManOffset { man, target, split, per });
                None
            }
            // BaseKagManager.TagUpdateMaidPoint: kInitMaidPoint (0) + the
            // maid points of the owned trophies.
            "updatemaidpoint" => {
                let (man, slot) = who(a);
                if !man {
                    let mut tjs = self.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
                    let state = &mut tjs.runtime.host_mut().state;
                    let data = state.data()?;
                    let bonus = data.trophy_bonus_maid_point(state.player.have_trophy.keys().copied());
                    if let Some(m) = state.maid_mut(slot) {
                        crate::maid_param::MaidParam { st: m, data: &data }.set_maid_point(bonus);
                    }
                }
                None
            }
            "setbg" => {
                self.ui.push(AdvUi::SetBg(property(a, name, "file")?.to_owned()));
                None
            }
            "charavisible" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::CharaVisible { man, slot, visible: a.contains_key("true") });
                None
            }
            "charavisiblealloff" => {
                self.ui.push(AdvUi::CharaVisibleAllOff { man: a.contains_key("man") });
                None
            }
            "charaactivate" => {
                let (man, slot) = who(a);
                let npc = a.get("npc").cloned();
                let stock = a.get("pc").map(|v| crate::text::atoi(v.as_bytes()).max(0) as usize);
                if self.retail_sound.is_some() {
                    let mut tjs = self.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
                    let state = &mut tjs.runtime.host_mut().state;
                    let named = !state.retail.compatibility && ["leader","rank1","main"].iter().any(|key|a.contains_key(*key));
                    if !state.retail.compatibility { self.ui.push(AdvUi::StopMotionScripts); }
                    if !named && let Some(name) = npc {
                        self.ui.push(AdvUi::RetailNpc {man,slot,name,load:!a.contains_key("noload")});
                    } else if !named && !a.contains_key("slot") && !a.contains_key("sub") && let Some(other) = a.get("swap") {
                        self.ui.push(AdvUi::RetailSwap {man,slot,other:other.parse()?});
                    } else {
                        anyhow::ensure!(!man && slot < 18,"invalid retail stock destination");
                        if let Some(index) = crate::tjs::retail::select_stock(state,a)? {
                            self.ui.push(AdvUi::RetailStock {index,slot,visible:(!a.contains_key("noload")).then_some(true)});
                            for active in &mut state.active_maids { if *active == Some(index) { *active=None; } }
                            state.active_maids.resize(state.active_maids.len().max(slot+1),None);
                            state.active_maids[slot]=Some(index);
                        }
                    }
                    // Commit the CharacterMgr effects before the next tag can query the slots.
                    self.exec_requested=true;
                    return Ok(Some(KagStep::Running));
                }
                anyhow::ensure!(npc.is_some() || stock.is_some(), "@charaactivate needs npc or pc");
                self.ui.push(AdvUi::CharaActivate { man, slot, npc, stock });
                None
            }
            "charadeactivate" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::CharaDeactivate { man, slot });
                if self.retail_sound.is_some() {
                    self.exec_requested=true;
                    return Ok(Some(KagStep::Running));
                }
                None
            }
            "camera" => {
                if a.contains_key("reset") {
                    self.ui.push(AdvUi::CameraReset);
                } else {
                    self.ui.push(AdvUi::Camera {
                        target: vec3(a, ["cx", "cy", "cz"]).unwrap_or([0.; 3]),
                        distance: real(a, "radius").unwrap_or(0.),
                        around: [real(a, "rx").unwrap_or(0.), real(a, "ry").unwrap_or(0.)],
                    });
                }
                None
            }
            "cameraanimationstart" => {
                let to = (
                    vec3(a, ["cx", "cy", "cz"]).unwrap_or([0.; 3]),
                    real(a, "radius").unwrap_or(0.),
                    [real(a, "rx").unwrap_or(0.), real(a, "ry").unwrap_or(0.)],
                );
                let from = vec3(a, ["scx", "scy", "scz"]).map(|t| (t, real(a, "sradius").unwrap_or(to.1), [real(a, "srx").unwrap_or(0.), real(a, "sry").unwrap_or(0.)]));
                let ms = |k: &str| a.get(k).map(|v| crate::text::atoi(v.as_bytes()).max(0) as u64).unwrap_or(0);
                let ease = a.get("easetype").cloned().unwrap_or_else(|| "linear".into());
                self.ui.push(AdvUi::CameraAnimation { from, to, delay_ms: ms("delay"), ms: ms("time"), ease });
                None
            }
            "lightmain" => {
                self.ui.push(AdvUi::LightMain { rotation: vec3(a, ["rx", "ry", "rz"]), intensity: real(a, "intensity") });
                None
            }
            // BaseKagManager.TagMotionScript (immediate form).
            "motionscript" => {
                let file = format!("{}.ks", property(a, name, "file")?);
                let slot = a.get("sloat").map(|v| crate::text::atoi(v.as_bytes())).unwrap_or(0);
                let label = a.get("label").cloned().unwrap_or_default();
                let op = if self.retail_sound.is_some() {
                    let actor = |key:&str| a.get(key).map(|v| usize::try_from(crate::text::atoi(v.as_bytes()))).transpose();
                    AdvUi::RetailMotionScript {slot,file,label,maid:actor("maid")?,man:actor("man")?,positions:!a.contains_key("npos")}
                } else { AdvUi::MotionScript { slot, file, label } };
                let wait = a.get("wait").map(|v| crate::text::atoi(v.as_bytes())).unwrap_or(0);
                if wait <= 0 {
                    self.ui.push(op);
                    self.clear_events("motion_script");
                } else {
                    self.add_event("motion_script", now, wait as u64, op);
                }
                None
            }
            // TagMotion / TagMotionNext (queued after the current clip;
            // after pending motion events with the last one's delay).
            "motion" | "motionnext" => {
                let (man, slot) = who(a);
                let file = format!("{}.anm", property(a, name, "mot")?);
                let looping = a.get("loop").is_some_and(|v| as_bool(v));
                let blend_s = a.get("blend").map(|v| crate::text::atoi(v.as_bytes())).unwrap_or(500) as f32 / 1000.;
                // MotionKagManager.PlayMaidMotion: next_flag queues once.
                let queue = name == "motionnext" || std::mem::take(&mut self.next_motion);
                let op = AdvUi::Motion { man, slot, file, looping, blend_s, queue };
                // ADVKagManager.PlayMaidMotion: StopMotionScript and, unless
                // queued, the pending `motion_script` events go.
                if self.kind == KagKind::Adv {
                    if !queue {
                        self.clear_events("motion_script");
                    }
                    self.ui.push(AdvUi::StopMotionScripts);
                }
                if name == "motionnext" {
                    match self.last_delay("motion") {
                        None => {
                            self.ui.push(op);
                            self.clear_events("motion");
                        }
                        Some(d) => self.add_event("motion", now, d, op),
                    }
                } else {
                    let wait = a.get("wait").map(|v| crate::text::atoi(v.as_bytes())).unwrap_or(0);
                    if wait <= 0 {
                        self.ui.push(op);
                        self.clear_events("motion");
                    } else {
                        self.add_event("motion", now, wait as u64, op);
                    }
                }
                None
            }
            "motionstop" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::MotionStop { man, slot });
                self.clear_events("motion");
                None
            }
            "motionlayerstop" => {
                let (man, slot) = who(a);
                let mut layers = Vec::new();
                if let Some(l) = a.get("layer") {
                    for x in l.split(',') {
                        layers.push(x.trim().parse::<i32>().with_context(|| format!("layer {x}"))?);
                    }
                }
                if let Some(r) = a.get("range") {
                    let (lo, hi) = r.split_once('-').context("motionlayerstopのrange設定が不正です.個数不一致")?;
                    let (lo, hi) = (lo.trim().parse::<i32>()?, hi.trim().parse::<i32>()?);
                    anyhow::ensure!(lo <= hi, "motionlayerstopのrange設定が不正です.範囲不一致");
                    layers.extend(lo..=hi);
                }
                self.ui.push(AdvUi::MotionLayerStop { man, slot, layers });
                None
            }
            "motionautotwist" => {
                let (man, slot) = who(a);
                let mut twists = Vec::new();
                for (both, pair) in [("kata", [("kata_r", "ShoulderR"), ("kata_l", "ShoulderL")]), ("tekubi", [("tekubi_r", "WristR"), ("tekubi_l", "WristL")]), ("momo", [("momo_r", "ThighR"), ("momo_l", "ThighL")])] {
                    if let Some(v) = a.get(both) {
                        let on = v == "on";
                        twists.push((pair[1].1, on));
                        twists.push((pair[0].1, on));
                    } else {
                        for (k, kind) in pair {
                            if let Some(v) = a.get(k) {
                                twists.push((kind, v == "on"));
                            }
                        }
                    }
                }
                self.ui.push(AdvUi::AutoTwist { man, slot, twists });
                None
            }
            "hairphisics" => {
                let (man, slot) = who(a);
                let mode = property(a, name, "hairr")?.to_owned();
                anyhow::ensure!(mode == "normal" || mode == "nofront", "hairrの指定[{mode}]は間違っています");
                self.ui.push(AdvUi::HairPhysics { man, slot, mode });
                None
            }
            "bustmove" => {
                let (man, slot) = who(a);
                let (left, right) = match property(a, name, "move")? {
                    "stop" => (Some(0.), Some(0.)),
                    "lstop" => (Some(0.), None),
                    "rstop" => (None, Some(0.)),
                    "non" => (Some(1.), Some(1.)),
                    _ => (None, None),
                };
                self.ui.push(AdvUi::BustMove { man, slot, left, right });
                None
            }
            "phisicshit" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::PhysicsHit { man, slot, height: crate::text::atof(property(a, name, "height")?.as_bytes()) });
                None
            }
            "ikattachpoint" | "ikattachbone" | "ikdetach" => {
                let src = who_str(property(a, name, "src")?)?;
                let bone = property(a, name, "srcbone")?.to_owned();
                let offset = [real(a, "offsetx").unwrap_or(0.), real(a, "offsety").unwrap_or(0.), real(a, "offsetz").unwrap_or(0.)];
                let target = match name {
                    "ikattachpoint" => Some((
                        who_str(property(a, name, "target")?)?,
                        IkTarget::AttachPoint { slot: property(a, name, "targetobj")?.to_owned(), point: property(a, name, "targetpoint")?.to_owned() },
                    )),
                    "ikattachbone" => Some((who_str(property(a, name, "target")?)?, IkTarget::Bone(property(a, name, "targetbone")?.to_owned()))),
                    _ => None,
                };
                self.ui.push(AdvUi::Ik { src, bone, target, offset });
                None
            }
            "alloffset" => {
                self.ui.push(AdvUi::AllOffset { position: vec3(a, ["x", "y", "z"]), rotation: vec3(a, ["rx", "ry", "rz"]) });
                None
            }
            "allresetpos" => {
                self.ui.push(AdvUi::AllResetPos);
                None
            }
            "camerachase" => {
                let (man, slot) = who(a);
                let bone = property(a, name, "chase")?.to_owned();
                let offset = [real(a, "offsx").unwrap_or(0.), real(a, "offsy").unwrap_or(0.), real(a, "offsz").unwrap_or(0.)];
                let seconds = a.get("time").map(|v| crate::text::atoi(v.as_bytes())).unwrap_or(0) as f32 / 1000.;
                self.ui.push(AdvUi::CameraChase { man, slot, bone, offset, once: !a.contains_key("continue"), seconds });
                None
            }
            "camerachasestop" => {
                self.ui.push(AdvUi::CameraChaseStop);
                None
            }
            "playvoice" => {
                let who = who(a);
                let file = crate::sound::ogg(property(a, name, "voice")?);
                let fade_s = a.get("fade").map(|v| crate::text::atoi(v.as_bytes())).unwrap_or(0) as f32 / 1000.;
                self.ui.push(AdvUi::Voice { who: Some(who), file, fade_s, looping: a.contains_key("loop") });
                None
            }
            "stopvoice" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::VoiceStop { man, slot });
                None
            }
            "stopsound" => {
                self.ui.push(AdvUi::StopSound);
                None
            }
            "itemchange" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::ItemChange { man, slot, category: property(a, name, "category")?.to_owned(), name: property(a, name, "name")?.to_owned() });
                None
            }
            "itemset" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::ItemSet { man, slot, category: property(a, name, "category")?.to_owned(), item: property(a, name, "item")?.to_owned(), temp: a.contains_key("temp") });
                None
            }
            "itemreset" => {
                let (man, slot) = who(a);
                let props: Vec<String> = if a.contains_key("allwear") {
                    crate::chara::MPN[39..=69].iter().map(|s| s.to_string()).collect()
                } else if a.contains_key("set") {
                    crate::chara::MPN[70..=72].iter().map(|s| s.to_string()).collect()
                } else if let Some(m) = a.get("multi") {
                    m.split(':').map(str::to_owned).collect()
                } else {
                    vec![property(a, name, "category")?.to_owned()]
                };
                self.ui.push(AdvUi::ItemReset { man, slot, props });
                None
            }
            // The item processing runs to completion when executed, so the
            // `sync` wait (SetMaidAllPropSeqWait) is already satisfied.
            "allprocpropseqstart" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::AllProcProp { man, slot });
                None
            }
            "itemmask" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::ItemMask { man, slot, body_slot: property(a, name, "slot")?.to_owned(), visible: as_bool(property(a, name, "hide")?) });
                None
            }
            "itemmaskmode" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::ItemMaskMode { man, slot, mode: property(a, name, "type")?.to_owned() });
                None
            }
            "chinko" => {
                let (man, slot) = who(a);
                if man {
                    self.ui.push(AdvUi::Chinko { slot, on: a.contains_key("on") });
                }
                None
            }
            "faceblend" => {
                let (man, slot) = who(a);
                let mut n = property(a, name, "name")?.to_owned();
                if n == "なし" {
                    n = "無し".into();
                }
                self.ui.push(AdvUi::FaceBlend { man, slot, name: n });
                None
            }
            "openmouth" => {
                let (man, slot) = who(a);
                if a.contains_key("open") {
                    self.ui.push(AdvUi::OpenMouth { man, slot, open: true });
                }
                if a.contains_key("close") {
                    self.ui.push(AdvUi::OpenMouth { man, slot, open: false });
                }
                None
            }
            "lipsync" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::Lipsync { man, slot, on: a.contains_key("on") });
                None
            }
            "startkuchipakupattern" => {
                let (man, slot) = who(a);
                let offset = crate::text::atof(property(a, name, "offsettime")?.as_bytes());
                self.ui.push(AdvUi::Kuchipaku { man, slot, pattern: Some((offset, property(a, name, "pattern")?.to_owned())) });
                None
            }
            "stopkuchipakupattern" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::Kuchipaku { man, slot, pattern: None });
                None
            }
            "eyetoposition" | "eyetotarget" | "eyetocamera" | "eyetoreset" => {
                let (man, slot) = who(a);
                let blend_s = a.get("blend").map(|v| crate::text::atoi(v.as_bytes())).unwrap_or(500) as f32 / 1000.;
                let look = match name {
                    "eyetoposition" => EyeLook::Position([real(a, "x").unwrap_or(0.), real(a, "y").unwrap_or(0.), real(a, "z").unwrap_or(0.)]),
                    "eyetotarget" => EyeLook::Chara { who: who_str(property(a, name, "target")?)?, bone: a.get("bone").cloned().filter(|b| !b.is_empty()) },
                    "eyetocamera" => EyeLook::Camera(property(a, name, "move")?.to_owned()),
                    _ => EyeLook::Reset,
                };
                self.ui.push(AdvUi::Eye { man, slot, look, blend_s });
                None
            }
            "setmaidoffsetposcategory" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::MaidOffsetCategory { man, slot, category: property(a, name, "category")?.to_owned(), offset: [real(a, "x"), real(a, "y"), real(a, "z")] });
                None
            }
            "addprefabbg" => {
                self.ui.push(AdvUi::AddPrefabBg {
                    src: property(a, name, "src")?.to_owned(),
                    name: property(a, name, "name")?.to_owned(),
                    dest: a.get("dest").cloned().unwrap_or_default(),
                    position: [real(a, "x").unwrap_or(0.), real(a, "y").unwrap_or(0.), real(a, "z").unwrap_or(0.)],
                    rotation: [real(a, "rx").unwrap_or(0.), real(a, "ry").unwrap_or(0.), real(a, "rz").unwrap_or(0.)],
                });
                None
            }
            "delprefabbg" => {
                self.ui.push(AdvUi::DelPrefabBg(Some(property(a, name, "name")?.to_owned())));
                None
            }
            "delprefabbgall" => {
                self.ui.push(AdvUi::DelPrefabBg(None));
                None
            }
            "moveprefabtochara" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::MovePrefabToChara {
                    man,
                    slot,
                    name: property(a, name, "name")?.to_owned(),
                    bone: property(a, name, "bone")?.to_owned(),
                    offset: [real(a, "offsx").unwrap_or(0.), real(a, "offsy").unwrap_or(0.), real(a, "offsz").unwrap_or(0.)],
                    seconds: a.get("time").map(|v| crate::text::atoi(v.as_bytes())).unwrap_or(0) as f32 / 1000.,
                });
                None
            }
            // Empty in 1.00 (TagAddPrefabWorld, TagReleasePrefab,
            // TagSetLoacalPosition, TagAddLocalPosition).
            "addprefabworld" | "releaseprefab" | "setloacalposition" | "addlocalposition" => None,
            "addprefabchara" => {
                let (man, slot) = who(a);
                let src = property(a, name, "src")?.to_owned();
                self.ui.push(AdvUi::AddPrefabChara {
                    man,
                    slot,
                    name: a.get("name").cloned().unwrap_or_else(|| src.clone()),
                    src,
                    bone: property(a, name, "bone")?.to_owned(),
                    position: [real(a, "x").unwrap_or(0.), real(a, "y").unwrap_or(0.), real(a, "z").unwrap_or(0.)],
                    rotation: [real(a, "rx").unwrap_or(0.), real(a, "ry").unwrap_or(0.), real(a, "rz").unwrap_or(0.)],
                });
                None
            }
            "delprefabchara" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::DelPrefabChara { man, slot, name: Some(property(a, name, "name")?.to_owned()) });
                None
            }
            "delprefabcharaall" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::DelPrefabChara { man, slot, name: None });
                None
            }
            "animobjchara" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::AnimObjChara {
                    man,
                    slot,
                    name: property(a, name, "name")?.to_owned(),
                    anim: property(a, name, "mot")?.to_owned(),
                    play: as_bool(property(a, name, "play")?),
                    looping: as_bool(property(a, name, "loop")?),
                });
                None
            }
            "texmuladd" => {
                let (man, slot) = who(a);
                for k in ["layer", "x", "y", "r", "s"] {
                    property(a, name, k)?;
                }
                let file = a.get("file").or_else(|| a.get("res")).context("@texmuladd [file] or [res] error.")?;
                let options: Vec<&str> = file.split(':').collect();
                let pick = self.random(0, options.len() as i32) as usize;
                let mut file = options[pick].to_owned();
                if a.contains_key("res") && !a.contains_key("file") {
                    file = format!("res:{file}");
                }
                let range = |k: &str| -> Result<(String, String)> {
                    let (lo, hi) = a[k].split_once(':').with_context(|| format!("@texmuladd [{k}] error."))?;
                    Ok((lo.to_owned(), hi.to_owned()))
                };
                let (xl, xh) = range("x")?;
                let x = self.random(crate::text::atoi(xl.as_bytes()), crate::text::atoi(xh.as_bytes()) + 1);
                let (yl, yh) = range("y")?;
                let y = self.random(crate::text::atoi(yl.as_bytes()), crate::text::atoi(yh.as_bytes()) + 1);
                let (rl, rh) = range("r")?;
                let r = self.random((crate::text::atof(rl.as_bytes()) * 100.) as i32, (crate::text::atof(rh.as_bytes()) * 100. + 1.) as i32) as f32 / 100.;
                let (sl, sh) = range("s")?;
                let sc = self.random((crate::text::atof(sl.as_bytes()) * 100.) as i32, (crate::text::atof(sh.as_bytes()) * 100. + 1.) as i32) as f32 / 100.;
                let op = AdvUi::TexMulAdd {
                    man,
                    slot,
                    body_slot: a.get("slot").cloned().unwrap_or_else(|| "body".into()),
                    material: a.get("matno").map(|v| crate::text::atoi(v.as_bytes()).max(0) as usize).unwrap_or(0),
                    property: a.get("propname").cloned().unwrap_or_else(|| "_MainTex".into()),
                    layer: crate::text::atoi(a["layer"].as_bytes()),
                    file,
                    at: (x, y, r, sc),
                };
                let delay = a.get("delay").map(|v| crate::text::atoi(v.as_bytes())).unwrap_or(0);
                if delay <= 0 {
                    // ExecForcWaitEvent: pending layers apply first.
                    if let Some(pending) = self.wait_events.remove("tex_mul_add") {
                        self.ui.extend(pending.into_iter().map(|e| e.2));
                    }
                    self.ui.push(op);
                } else {
                    self.add_event("tex_mul_add", now, delay as u64, op);
                }
                None
            }
            "texmuldel" => {
                let (man, slot) = who(a);
                self.clear_events("tex_mul_add");
                self.ui.push(AdvUi::TexMulDel {
                    man,
                    slot,
                    body_slot: a.get("slot").cloned().unwrap_or_else(|| "body".into()),
                    material: a.get("matno").map(|v| crate::text::atoi(v.as_bytes()).max(0) as usize).unwrap_or(0),
                    property: a.get("propname").cloned().unwrap_or_else(|| "_MainTex".into()),
                });
                None
            }
            "startday" => {
                self.ui.push(AdvUi::StartDay);
                None
            }
            "endday" => {
                self.ui.push(AdvUi::EndDay);
                None
            }
            // ADVKagManager choices.
            "choicesset" | "choicesrandomset" => {
                let text = property(a, name, "text")?.to_owned();
                let label = property(a, name, "label")?.to_owned();
                if name == "choicesset" {
                    self.choices.push((text, label, a.get("enabled").is_none_or(|v|as_bool(v))));
                } else {
                    self.random_choices.push((text, label, a.get("enabled").is_none_or(|v|as_bool(v))));
                }
                None
            }
            "choicesrandomselect" => {
                let n = (crate::text::atoi(property(a, name, "num")?.as_bytes()).max(0) as usize).min(self.random_choices.len());
                // Fisher–Yates as in TagChoicesRandomSelect.
                let mut i = self.random_choices.len();
                while i > 1 {
                    i -= 1;
                    let j = self.random(0, i as i32 + 1) as usize;
                    self.random_choices.swap(i, j);
                }
                let picked: Vec<_> = self.random_choices.drain(..).take(n).collect();
                self.choices.extend(picked);
                None
            }
            "_choicesshow" => {
                let choices=std::mem::take(&mut self.choices);
                self.ui.push(if self.retail_sound.is_some(){AdvUi::RetailChoices(choices)}else{AdvUi::Choices(choices.into_iter().map(|(text,label,_)|(text,label)).collect())});
                None
            }
            // KagScript::TagFuncR_Call / R_return (cm3d2_x64.dll): a
            // separate stack of (caller, stem, id); the return jumps to
            // `*RET_<stem>[_<id>]` in the caller.
            "r_call" => {
                let file = property(a, name, "file")?;
                let stem = file.rsplit_once('.').map_or(file, |(s, _)| s).to_owned();
                let id = a.get("id").map(|v| crate::text::atoi(v.as_bytes())).unwrap_or(-1);
                let caller = self.parser.cur_storage().unwrap_or("").to_owned();
                self.r_stack.push((caller, stem, id));
                self.pending_load = Some((scenario_name(file), None));
                None
            }
            "r_return" => {
                let (caller, stem, id) = self.r_stack.pop().context("R_return without R_call")?;
                if self.kind == KagKind::Adv {
                    self.tjs.lock().map_err(|_|anyhow::anyhow!("TJS lock"))?.runtime.host_mut().state.retail
                        .returned_to((self.parser.call_stack_depth(),self.r_stack.len()));
                }
                let label = if id >= 0 { format!("*RET_{stem}_{id}") } else { format!("*RET_{stem}") };
                self.pending_load = Some((caller, Some(label)));
                None
            }
            "allpos" => {
                self.ui.push(AdvUi::AllPos { position: vec3(a, ["x", "y", "z"]), rotation: vec3(a, ["rx", "ry", "rz"]) });
                None
            }
            "offset" => {
                let (man, slot) = who(a);
                self.ui.push(AdvUi::Offset { man, slot, position: vec3(a, ["x", "y", "z"]), rotation: vec3(a, ["rx", "ry", "rz"]) });
                None
            }
            "face" => {
                let (man, slot) = who(a);
                let op = AdvUi::Face { man, slot, name: property(a, name, "name")?.to_owned() };
                let wait = a.get("wait").map(|v| crate::text::atoi(v.as_bytes())).unwrap_or(0);
                if wait <= 0 {
                    self.ui.push(op);
                    self.clear_events("face");
                } else {
                    self.add_event("face", now, wait as u64, op);
                }
                None
            }
            // ADVKagManager.TagTalk: open the window, keep the speaker.
            "talk" => {
                self.ui.push(AdvUi::Window(true));
                self.talk_name = a.get("name").cloned().unwrap_or_default();
                // GetVoiceTargetMaid: maid=, then real=/name= by chara name
                // (resolved by the adapter).
                self.talk_voice = a.get("voice").map(|v| {
                    let target = a.get("maid").map(|m| (false, crate::text::atoi(m.as_bytes()).max(0) as usize));
                    (format!("{v}.ogg"), target)
                });
                self.talk_real = a.get("real").or_else(|| a.get("name")).cloned();
                None
            }
            // ADVKagManager.TagHitRet: publish the text, wait for the click.
            "hitret" => {
                let mut text = std::mem::take(&mut self.text);
                while text.chars().count() > 1 && text.ends_with(['|', ' ', '\u{3000}']) {
                    text.pop();
                }
                let text = text.replace('|', "\n");
                self.ui.push(AdvUi::Message { name: std::mem::take(&mut self.talk_name), text });
                if let Some((file, maid)) = self.talk_voice.take() {
                    self.ui.push(AdvUi::TalkVoice { file, maid, chara_name: self.talk_real.take() });
                }
                self.hitret_wait = true;
                self.hitret_at = now;
                Some(KagStep::Waiting)
            }
            "messagewindow" => {
                if a.contains_key("on") {
                    self.ui.push(AdvUi::Window(true));
                } else if a.contains_key("off") {
                    self.ui.push(AdvUi::Window(false));
                }
                None
            }
            "cameracontrol" => {
                self.ui.push(AdvUi::CameraControl(a.contains_key("true")));
                None
            }
            "fade" | "fade3d" => {
                let seconds = crate::text::atoi(property(a, name, "time")?.as_bytes()) as f32 / 1000.;
                let to = if a.contains_key("in") { 0. } else { 1. };
                if name == "fade" {
                    self.motion_blend = to == 0.;
                    self.ui.push(AdvUi::MotionBlend(self.motion_blend));
                    self.ui.push(AdvUi::Fade { to, seconds });
                    self.ui.push(AdvUi::ClearText);
                } else {
                    self.ui.push(AdvUi::Fade3d { to, seconds });
                }
                if a.contains_key("sync") {
                    self.fade_wait = true;
                    Some(KagStep::Waiting)
                } else {
                    None
                }
            }
            // BaseKagManager.TagPlayBgm/TagPlayEnv: fade in milliseconds.
            "playbgm" | "playenv" => {
                use crate::sound::{ogg, Kind, SoundOp};
                let file = ogg(property(a, name, "file")?);
                let fade = a.get("fade").map(|f| crate::text::atoi(f.as_bytes())).unwrap_or(0) as f32 / 1000.;
                let kind = if name == "playbgm" { Kind::Bgm } else { Kind::Env };
                self.sound.push(SoundOp::Play { kind, file, fade_s: fade, looping: true });
                None
            }
            "stopbgm" | "stopenv" => {
                use crate::sound::{Kind, SoundOp};
                let fade = a.get("fade").map(|f| crate::text::atoi(f.as_bytes())).unwrap_or(0) as f32 / 1000.;
                let kind = if name == "stopbgm" { Kind::Bgm } else { Kind::Env };
                self.sound.push(SoundOp::Stop { kind, fade_s: fade });
                None
            }
            // BaseKagManager.TagPlaySe without `wait`; delayed SEs use the
            // wait-event list (SATD-cm3d2-game-logic).
            "playse" => {
                use crate::sound::{ogg, Kind, SoundOp};
                let file = ogg(property(a, name, "file")?);
                let op = SoundOp::Play { kind: Kind::Se, file, fade_s: 0., looping: a.contains_key("loop") };
                let wait = a.get("wait").map(|v| crate::text::atoi(v.as_bytes())).unwrap_or(0);
                if wait <= 0 {
                    self.sound.push(op);
                    self.clear_events("se");
                } else {
                    self.add_event("se", now, wait as u64, AdvUi::Sound(op));
                }
                None
            }
            "stopse" => {
                self.sound.push(crate::sound::SoundOp::Stop { kind: crate::sound::Kind::Se, fade_s: 0. });
                None
            }
            // YotogiManager registers it on the ADV KAG.
            "yotogicall" if self.kind == KagKind::Adv => {
                let n = property(a, name, "name")?.to_owned();
                let label = property(a, name, "label")?.to_owned();
                self.ui.push(AdvUi::YotogiCall { name: n, label, edit_label: a.get("edit_label").cloned() });
                None
            }
            // SceneSingleEffect registers it while that scene runs.
            "startmvpeffect" if self.kind == KagKind::Adv => {
                self.ui.push(AdvUi::StartMvpEffect);
                None
            }
            "charaactivateleader" => {
                self.ui.push(AdvUi::StopMotionScripts);
                let slot = a.get("maid").map(|v| crate::text::atoi(v.as_bytes()).max(0) as usize).unwrap_or(0);
                self.ui.push(AdvUi::CharaActivateLeader { slot });
                None
            }
            // KAG `eval`: run the TJS expression.
            "eval" => {
                let exp = property(a, name, "exp")?;
                self.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock poisoned"))?.eval(exp)?;
                None
            }
            // KagScript.CallTag: a tag without a callback does nothing.
            other => {
                if let Some(t) = self.trace.as_mut() {
                    t.push(format!("unregistered [{other}]"));
                }
                None
            }
        })
    }

    /// The tags `YotogiKagManager` and `YotogiManager` register on
    /// `yotogi_kag`; `None` when the tag is not one of them.
    fn yotogi_tag(&mut self, now: u64, name: &str, a: &BTreeMap<String, String>) -> Result<Option<Option<KagStep>>> {
        let _ = now;
        Ok(Some(match name {
            // TagTalk: only the voice (GetVoiceTargetMaid + PlayVoice).
            "talk" => {
                if let Some(v) = a.get("voice") {
                    let maid = a.get("maid").map(|m| (false, crate::text::atoi(m.as_bytes()).max(0) as usize));
                    let chara_name = a.get("real").or_else(|| a.get("name")).cloned();
                    self.ui.push(AdvUi::TalkVoice { file: format!("{v}.ogg"), maid, chara_name });
                }
                None
            }
            "talkrepeat" => {
                if let Some(v) = a.get("voice") {
                    self.ui.push(AdvUi::RepeatVoice(format!("{v}.ogg")));
                }
                None
            }
            // SetWait(maid 0's AudioMan.GetLength(), skip_possible: true);
            // the adapter supplies the length.
            "voicewait" => {
                self.voice_wait = true;
                Some(KagStep::Waiting)
            }
            "hitret" => {
                self.text.clear();
                None
            }
            "playbgmroom" => {
                let fade = a.get("fade").map(|v| crate::text::atoi(v.as_bytes())).unwrap_or(0);
                self.ui.push(AdvUi::PlayBgmRoom { fade_s: fade as f32 / 1000. });
                None
            }
            "controlmask" => {
                self.ui.push(AdvUi::ControlMask);
                None
            }
            _ => return Ok(None),
        }))
    }
}
