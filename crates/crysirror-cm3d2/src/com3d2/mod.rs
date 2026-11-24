// SPDX-License-Identifier: MPL-2.0
//! Standalone Custom Order Maid 3D 2, Unity 5.6.4p2 / serialized format 17.
mod club;
mod editor;
pub(crate) mod stock_editor;
pub(crate) mod actors;
mod tracking;
mod choices;
mod quit_dialog;
mod trophy;
mod widgets;
use crate::{
    adapter::{Config, Opener},
    files::GameFiles,
    kag::{AdvKag, KagStep},
    sound::{Kind, Sound},
};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::{
    Error,
    service::{DesktopRequest, DesktopRequestJob, DesktopRequestState, DesktopResponse, HostServices, InputFrame, buttons},
    state::ClientId,
    vm::{VmAdapter, VmYield},
};
use serde_json::json;
use widgets::{Assets, Widget};

const BUTTONS: [&str; 6] = ["Start", "Continue", "Guest", "Config", "End", "Shop"];
const CLICK: u64 =
    buttons::ENTER | buttons::MOUSE_LEFT | buttons::MOUSE_RIGHT | buttons::MOUSE_MIDDLE;

#[derive(PartialEq, Clone, Copy, Debug)]
enum Scene {
    Logo,
    Warning,
    Script,
    Title,
    Setup,
    Editor,
    StartDaily,
    StockEditor,
}
impl Scene {
    fn name(self) -> &'static str {
        match self {
            Self::Logo => "SceneLogo",
            Self::Warning => "SceneWarning",
            Self::Script => "SceneADV",
            Self::Title => "SceneTitle",
            Self::StockEditor => "SceneEdit",
            Self::StartDaily => "SceneStartDaily",
            Self::Editor => "SceneUserEdit",
            Self::Setup => "SceneNetorareCheck",
        }
    }
}

#[derive(Debug, PartialEq, serde::Serialize)]
struct Preferences {
    lock_ntr_play: bool,
    lock_user_draft_maid: bool,
    is_old_player: bool,
}

// SceneADV animates the parent transform first, then fades the whole panel.
fn title_frame(ms: u64, hold: u64, width: f32) -> (f32, u8) {
    let enter = (ms as f32 / 500.).min(1.);
    let shift = -width + ((width - 50.) * (1. - (1. - enter).powi(3))).trunc();
    let fade = (ms.saturating_sub(500 + hold) as f32 / 500.).min(1.);
    (shift, ((1. - fade).powi(3) * 255.).round() as u8)
}

/// ScriptManager.ReplaceCharaName scans the five active slots in slot order.
fn retail_names(text: &str, state: &crate::tjs::TjsState) -> String {
    let mut out = text.to_owned();
    if !out.contains('[') { return out; }
    for slot in 0..5 {
        let Some(maid) = state.maid(slot) else { continue };
        let suffix = if slot == 0 { String::new() } else { slot.to_string() };
        let full = format!("{} {}", maid.last_name, maid.first_name);
        let call = if maid.is_first_name_call { &maid.first_name } else { &maid.last_name };
        for (stem, value) in [("HF", call.as_str()), ("HL", maid.last_name.as_str()), ("HLF", full.as_str())] {
            for token in [stem.to_ascii_lowercase(), stem.to_owned()] {
                let numbered = format!("[{token}{suffix}]");
                if out.contains(&numbered) {
                    let base = format!("[{token}]");
                    out = out.replace(&numbered, &base).replace(&base, value);
                }
            }
        }
    }
    out.replace("[SF]", &state.player.player_name).replace("[sf]", &state.player.player_name)
}

type Orbit = ([f32; 3], f32, [f32; 2]);
const DAY_VIEWS: [(&str, Orbit); 9] = [
    ("EmpireClub_Rotary", ([-17.42608, 1.62898, 18.58508], 0.7999998, [-227.0059, -10.18634])),
    ("EmpireClub_Entrance", ([4.084215, 5.501422, 11.13918], 1.1, [-165.0296, 11.24223])),
    ("MyRoom", ([5.273936, 2.515735, 3.353783], 0.1, [-110.838, 16.8323])),
    ("HeroineRoom_B1", ([2.837114, 2.19285, 3.352386], 0.1, [-128.5022, 12.48449])),
    ("HeroineRoom_A1", ([2.837114, 2.19285, 3.352386], 0.1, [-128.5022, 12.48449])),
    ("HeroineRoom_C1", ([2.837114, 2.19285, 3.352386], 0.1, [-128.5022, 12.48449])),
    ("ShinShitsumu", ([-2.828141, 2.77968, 4.463314], 0.1, [-210.2392, 18.07455])),
    ("Theater", ([-10.00124, 0.8031937, 3.407116], 0.1, [-311.137, 2.236061])),
    ("DanceRoom", ([0.7564614, 1.38443, -1.607882], 2.0, [-138.9819, 14.03727])),
];
struct AdvCameraMove {
    from: Orbit,
    to: Orbit,
    start: u64,
    duration: u64,
    ease: String,
}
struct Game {
    adv_camera: Orbit,
    camera_move: Option<AdvCameraMove>,
    adv_cover: Option<crate::adapter::Fade>,
    world_cover: Option<crate::adapter::Fade>,
    dialogue: Option<(String, String)>,
    editor: editor::Editor,
    stock_editor: stock_editor::StockEditor,
    actors: actors::Actors,
    assets: Assets,
    files: std::sync::Arc<GameFiles>,
    script: AdvKag,
    sound: Sound,
    scene: Scene,
    widgets: Vec<Widget>,
    title_banner: Option<(String, Option<u64>)>,
    message_open: bool,
    camera_control: bool,
    previous_layers: usize,
    club: Option<String>,
    club_meshes: usize,
    entered: u64,
    leaving: Option<u64>,
    down: u64,
    pointer: Option<[i32; 2]>,
    pressed: Option<usize>,
    focus: usize,
    quit_dialog: Option<quit_dialog::QuitDialog>,
    choices: Option<choices::Choices>,
    quit_confirmed: bool,
    day_camera_fade: Option<u64>,
    trophies: trophy::Effects,
    shop_requests: Vec<Box<dyn DesktopRequestJob>>,
}

impl Game {
    fn enter(
        &mut self,
        scene: Scene,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        now: u64,
    ) -> Result<()> {
        if matches!(self.scene,Scene::Script | Scene::StockEditor) && self.scene != scene {
            self.actors.leave(host, client)?;
        }
        if self.scene == Scene::Editor {
            self.editor.leave(host, client)?;
        }
        self.widgets = self.assets.scene(op, host, client, scene.name()).with_context(|| format!("load widgets for {}", scene.name()))?;
        if scene == Scene::Script {
            self.widgets.extend(self.assets.scene(op, host, client, "_cm3d21")?);
        }
        self.choices=None;
        self.title_banner = None;
        {
            let mut tjs = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let retail = &mut tjs.runtime.host_mut().state.retail;
            retail.scene_loaded(scene.name(), &self.script.tag_backup);
            retail.adv_present = scene == Scene::Script;
            retail.title_request = None;
        }
        if !(self.scene == Scene::StartDaily && scene == Scene::Script) && let Some(asset) = self.club.take() {
            host.submit_engine(
                client,
                1,
                &serde_json::to_vec(&json!({"world":[{"op":"despawn","id":1}]}))?,
            )?;
            host.release_model(
                client,
                &crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, &asset)?,
            )?;
            self.club_meshes = 0;
        }
        if matches!(scene, Scene::Title | Scene::Setup | Scene::Editor | Scene::StartDaily | Scene::StockEditor) {
            let (name, model, mut camera, environment) = if scene == Scene::Title {
                let name = "EmpireClub_Rotary_Night";
                let file = self.assets.scene_file(scene.name())?;
                (
                    name.to_owned(),
                    club::model(&mut self.assets.unity, op, &file, name)?,
                    crate::bg::orbit_camera(
                        [-9.30211, 1.579518, 17.12966],
                        [-221.6162, -9.254665],
                        0.5999951,
                        35.,
                    ),
                    json!({}),
                )
            } else if scene == Scene::StartDaily {
                self.message_open = false;
                self.camera_control = true;
                self.camera_move = None;
                self.adv_cover = None;
                self.world_cover = None;
                self.day_camera_fade = None;
                host.submit_engine(client, 1, &serde_json::to_vec(&json!({"layers":[{"op":"remove","id":65534}]}))?)?;
                let mut tjs = self.script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
                let state = &mut tjs.runtime.host_mut().state;
                self.actors.reset_for_day(state);
                let day = state.player.days.checked_add(1).context("day display overflow")?;
                self.assets.day_digits(op, host, client, day, &mut self.widgets)?;
                let (name, orbit) = DAY_VIEWS[state.random(DAY_VIEWS.len())];
                self.adv_camera = orbit;
                (name.to_owned(), club::prefab(&mut self.assets.unity, op, name)?,
                    crate::bg::orbit_camera(orbit.0, orbit.2, orbit.1, 35.),
                    json!({"show_sky":false,"tonemap":"none","sun":{"direction":[0.,-0.766,0.643],"color":[1.,1.,1.],"intensity":0.65}}))
            } else if scene == Scene::StockEditor {
                self.message_open=false;
                self.adv_cover=None;
                self.world_cover=None;
                self.camera_move=None;
                let mut tjs=self.script.tjs.lock().map_err(|_|anyhow::anyhow!("TJS lock"))?;
                let state=&mut tjs.runtime.host_mut().state;
                let name=self.stock_editor.open(state,&self.script.tag_backup,now).context("initialize stock editor")?;
                self.actors.reset_for_day(state);
                self.actors.show_editor_stock()?;
                let id=state.active_maids[0].context("empty edit slot")?;
                self.stock_editor.load_ui(&mut self.widgets,&mut self.assets,op,host,client,&state.retail.stock_models[&id]).context("load stock editor UI")?;
                let model=club::prefab(&mut self.assets.unity,op,&name).with_context(||format!("stock edit background {name}"))?;
                self.adv_camera=([-0.05539433,0.95894,0.1269088],3.,[-180.,11.5528]);
                (name,model,crate::bg::orbit_camera(self.adv_camera.0,self.adv_camera.2,3.,35.),
                  json!({"show_sky":false,"tonemap":"none","sun":{"direction":[0.,-0.766,0.643],"color":[1.,1.,1.],"intensity":0.65}}))
            } else if scene == Scene::Editor {
                let state = self
                    .script
                    .tjs
                    .lock()
                    .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                let night = state.runtime.host().state.player.flag("時間帯") == 3;
                let name = if night {
                    "ShinShitsumu_ChairRot_Night"
                } else {
                    "ShinShitsumu_ChairRot"
                };
                (
                    name.to_owned(),
                    club::prefab(&mut self.assets.unity, op, name)?,
                    crate::bg::orbit_camera_offset(
                        [0.08306749, 1.299064, 0.0249091],
                        [177.9044, 2.546583],
                        2.2,
                        35.,
                        320.,
                    ),
                    json!({"show_sky":false,"tonemap":"none","sun":{"direction":[0.,-0.766,0.643],"color":[1.,1.,1.],"intensity":0.65}}),
                )
            } else {
                self.message_open = false;
                self.camera_control = false;
                let name = "ShinShitsumu";
                (
                    name.to_owned(),
                    club::prefab(&mut self.assets.unity, op, name)?,
                    crate::bg::orbit_camera(
                        [0.5609447, 1.380762, -1.382336],
                        [245.5691, 6.273283],
                        1.6,
                        35.,
                    ),
                    json!({"show_sky":false,"tonemap":"none","sun":{"direction":[0.,-0.766,0.643],"color":[1.,1.,1.],"intensity":0.65}}),
                )
            };
            let (glb, count) = model;
            let asset = host
                .upload_model(client, 0x50000000, &glb)?
                .relative()
                .to_owned();
            camera["op"] = json!("set_camera");
            camera["camera"]["near"] = json!(0.01);
            let world = json!({"world": [
                {"op": "set_environment", "environment": environment},
                {"op": "spawn", "entity": {"id": 1, "name": name,
                    "geometry": {"kind": "scene", "asset": asset}}},
                camera
            ]});
            host.submit_engine(client, 1, &serde_json::to_vec(&world)?)?;
            self.script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?
                .runtime
                .host_mut()
                .state
                .bg_name = name.to_owned();
            self.club = Some(asset);
            self.club_meshes = count;
        }
        if scene == Scene::StockEditor {
            self.edit_pose(op,now)?;
            self.sound.play(host,client,&self.files,Kind::Bgm,"BGM005.ogg",1.,true)?;
        }
        if scene == Scene::Editor {
            let mut tjs = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            self.editor.enter(
                op,
                host,
                client,
                &self.files,
                &mut self.widgets,
                &self.assets.editor_layout,
                &mut tjs.runtime.host_mut().state,
                !self.script.tag_backup.contains_key("second"),
                now,
            )?;
            self.editor.help(
                &mut tjs.runtime.host_mut().state,
                &mut self.assets,
                op,
                host,
                client,
                &self.files,
                &mut self.widgets,
            )?;
            self.sound.play(
                host,
                client,
                &self.files,
                Kind::Bgm,
                &self.assets.editor_layout.bgm,
                0.5,
                true,
            )?;
        }
        self.pressed = None;
        self.scene = scene;
        self.entered = now;
        self.leaving = None;
        if scene == Scene::Title {
            self.sound
                .play(host, client, &self.files, Kind::Bgm, "BGM004.ogg", 0., true)?;
        }
        Ok(())
    }

    fn edit_pose(&mut self,op:&mut Opener,now:u64)->Result<()> {
        let (file,label)=&self.stock_editor.pose;
        let effects=self.actors.start_motion(&self.script,0,file,label,Some(0),None,true,&self.files,op,now)?;
        self.edit_motion_effects(effects,op,now)
    }

    fn edit_motion_effects(&mut self,effects:Vec<crate::kag::AdvUi>,op:&mut Opener,now:u64)->Result<()> {
        for effect in effects {
            if let crate::kag::AdvUi::MotionBlend(on)=effect {self.script.motion_blend=on;continue;}
            self.actors.apply(effect,&self.script,&self.files,&mut self.assets,op,now)?;
        }
        Ok(())
    }

    fn resume_script(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        now: u64,
    ) -> Result<()> {
        let label = self
            .script
            .tag_backup
            .get("label")
            .context("missing SceneCall return label")?
            .clone();
        self.script
            .jump_label(&self.files, op.storage.as_ref(), &label)?;
        self.script.enabled = true;
        self.script.exec();
        self.enter(Scene::Script, op, host, client, now)
    }

    fn button(w: &Widget) -> Option<usize> {
        let mut parts = w.path.split('/');
        parts.find(|s| *s == "ButtonGroup")?;
        let group = parts.next()?.strip_suffix("Group")?;
        BUTTONS.iter().position(|s| *s == group)
    }

    fn rect(scene: Scene, w: &Widget, elapsed: u64) -> [f32; 4] {
        let mut rect = w.rect;
        if scene == Scene::Title {
            if let Some(i) = Self::button(w) {
                let t = ((elapsed as f32 - i as f32 * 300.) / 700.).clamp(0., 1.);
                rect[0] -= 600. * (1. - crate::scenes::ease_out_expo(t));
            }
        }
        rect
    }

    fn update(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        op: &mut Opener,
        input: InputFrame,
    ) -> Result<()> {
        let now = host.now_ms();
        let elapsed = now.saturating_sub(self.entered);
        let released = self.down & !input.buttons_down;
        self.down = input.buttons_down;
        self.sound.poll(host, client)?;
        let mut i = 0;
        while i < self.shop_requests.len() {
            match self.shop_requests[i].poll()? {
                DesktopRequestState::Pending => i += 1,
                DesktopRequestState::Ready(DesktopResponse::Opened) => { self.shop_requests.swap_remove(i); }
                DesktopRequestState::Ready(_) => bail!("unexpected COM3D2 shop URL response"),
            }
        }
        match self.scene {
            Scene::Logo | Scene::Warning => {
                if let Some(since) = self.leaving {
                    if now.saturating_sub(since) >= 1000 {
                        if self.scene == Scene::Logo {
                            self.enter(Scene::Warning, op, host, client, now)?;
                        } else {
                            self.script
                                .load(&self.files, op.storage.as_ref(), "AllMain.ks", "")?;
                            self.script.exec();
                            self.enter(Scene::Script, op, host, client, now)?;
                        }
                    }
                } else if elapsed >= 1000 && released & CLICK != 0 {
                    self.sound.play(
                        host,
                        client,
                        &self.files,
                        Kind::System,
                        "SE002.ogg",
                        0.,
                        false,
                    )?;
                    self.leaving = Some(now);
                }
            }
            Scene::Script => {
                let step=if let Some(choices)=&mut self.choices {
                    if let Some(label)=choices.update(input,released,now).map(str::to_owned) {
                        self.script.choose(&self.files,op.storage.as_ref(),&label)?;
                        self.widgets.truncate(choices.first);
                        self.choices=None;
                    }
                    KagStep::Waiting
                }else{self.script.update(
                    now,
                    input.buttons_pressed & (buttons::ENTER | buttons::MOUSE_LEFT) != 0,
                    [self.adv_cover, self.world_cover].into_iter().flatten().any(|f| !f.done(now)),
                    &self.files,
                    op.storage.as_ref(),
                )?};
                if let Some(queue) = self.script.retail_sound.as_mut()
                    && !queue.is_empty()
                {
                    let tjs = self
                        .script
                        .tjs
                        .lock()
                        .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                    let state = &tjs.runtime.host().state.retail;
                    for (op, legacy) in queue.drain(..) {
                        use crate::sound::SoundOp;
                        match op {
                            SoundOp::Play {
                                kind,
                                file,
                                fade_s,
                                looping,
                            } => {
                                if let Some(files) =
                                    state.audio_files(&self.files, kind, &file, legacy)
                                {
                                    self.sound.play_profile(
                                        host, client, files, kind, &file, fade_s, looping, legacy,
                                    )?;
                                }
                            }
                            SoundOp::Stop { kind, fade_s } => {
                                self.sound.stop(host, client, kind, fade_s)?
                            }
                            SoundOp::Volume { kind, value } => {
                                self.sound.set_volume(host, client, kind, value)?
                            }
                        }
                    }
                }
                // During reset_scene.ks these refer only to the empty boot ADV stage.
                let mut effects = std::collections::VecDeque::from(std::mem::take(&mut self.script.ui));
                effects.extend(self.actors.tick_motions(&self.files,op,now)?);
                while let Some(effect) = effects.pop_front() {
                    use crate::kag::AdvUi;
                    match effect {
                        AdvUi::CharaVisible {
                            man: true,
                            slot: 0,
                            visible: false,
                        } => self.editor.hide_player(),
                        AdvUi::CharaVisibleAllOff { man: false }
                            if self.script.tjs.lock().is_ok_and(|t| {
                                t.runtime
                                    .host()
                                    .state
                                    .active_maids
                                    .iter()
                                    .all(Option::is_none)
                            }) => {}
                        AdvUi::RetailChoices(mut options) => {
                            let vm = self.script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
                            for (title, _, _) in &mut options {
                                *title = retail_names(title, &vm.runtime.host().state);
                            }
                            drop(vm);
                            let template=self.assets.scene(op,host,client,"SelectButton")?;
                            self.choices=Some(choices::Choices::show(&mut self.widgets,template,options,&self.assets.choice_layout)?);
                        }
                        AdvUi::StartDay => {
                            let mut vm = self.script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
                            crate::tjs::retail::start_before_hiring(&mut vm.runtime.host_mut().state)?;
                        }
                        AdvUi::ClearText => self.dialogue = None,
                        AdvUi::Message { name, text } => {
                            let t = self
                                .script
                                .tjs
                                .lock()
                                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                            let state = &t.runtime.host().state;
                            self.dialogue = Some((retail_names(&name, state), retail_names(&text, state)));
                        }
                        AdvUi::Fade { to, seconds } | AdvUi::Fade3d { to, seconds } => {
                            let cover = if matches!(effect, AdvUi::Fade3d { .. }) { &mut self.world_cover } else { &mut self.adv_cover };
                            *cover = Some(crate::adapter::Fade {
                                from: cover.map_or(0., |f| f.value(now)),
                                to,
                                start_ms: now,
                                duration_ms: (seconds * 1000.).max(0.) as u64,
                            });
                        }
                        AdvUi::SetBg(name) => {
                            let (glb, count) = club::prefab(&mut self.assets.unity, op, &name)?;
                            if let Some(old) = self.club.take() {
                                host.submit_engine(
                                    client,
                                    1,
                                    &serde_json::to_vec(
                                        &json!({"world":[{"op":"despawn","id":1}]}),
                                    )?,
                                )?;
                                host.release_model(
                                    client,
                                    &crysirror_core::path::VfsPath::new(
                                        crysirror_core::path::Mount::Asset,
                                        &old,
                                    )?,
                                )?;
                            }
                            let asset = host
                                .upload_model(client, 0x50000000, &glb)?
                                .relative()
                                .to_owned();
                            host.submit_engine(client,1,&serde_json::to_vec(&json!({"world":[{"op":"spawn","entity":{"id":1,"name":name,"geometry":{"kind":"scene","asset":asset}}}]}))?)?;
                            self.script.tjs.lock().map_err(|_|anyhow::anyhow!("TJS lock"))?.runtime.host_mut().state.bg_name = name;
                            self.club = Some(asset);
                            self.club_meshes = count;
                        }
                        AdvUi::Camera {
                            target,
                            distance,
                            around,
                        } => {
                            self.adv_camera = (target, distance, around);
                            self.camera_move = None;
                            let mut cmd = crate::bg::orbit_camera(target, around, distance, 35.);
                            cmd["op"] = json!("set_camera");
                            host.submit_engine(
                                client,
                                1,
                                &serde_json::to_vec(&json!({"world":[cmd]}))?,
                            )?;
                        }
                        AdvUi::CameraAnimation {
                            from,
                            to,
                            delay_ms,
                            ms,
                            ease,
                        } => {
                            self.camera_move = Some(AdvCameraMove {
                                from: from.unwrap_or(self.adv_camera),
                                to,
                                start: now + delay_ms,
                                duration: ms,
                                ease,
                            });
                        }
                        AdvUi::Window(open) => self.message_open = open,
                        AdvUi::CameraControl(enabled) => self.camera_control = enabled,
                        AdvUi::RetailMotionScript { slot, file, label,maid,man,positions } => {
                            let ops = self.actors.start_motion(&self.script, slot, &file, &label, maid, man, positions, &self.files, op, now)?;
                            for effect in ops.into_iter().rev() { effects.push_front(effect); }
                        }
                        AdvUi::RetailTrophy(id)=>self.trophies.pending.push_back(id),
                        AdvUi::RetailClearMotionStacks=>self.actors.clear_motion_stacks(),
                        AdvUi::StopMotionScripts => self.actors.motions.clear(),
                        AdvUi::MotionBlend(on) => self.script.motion_blend = on,
                        AdvUi::TalkVoice { file, .. } => {
                            self.sound.stop(host, client, Kind::Voice, 0.)?;
                            let vm = self.script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
                            let state = &vm.runtime.host().state.retail;
                            if let Some(files) = state.audio_files(&self.files, Kind::Voice, &file, state.compatibility) {
                                self.sound.play_profile(host, client, files, Kind::Voice, &file, 0., false, state.compatibility)?;
                            }
                        }
                        AdvUi::LightMain { rotation, intensity } => {
                            let d = crate::chara::unity_euler(rotation.unwrap_or([50.,0.,0.])) * glam::Vec3::Z;
                            host.submit_engine(client,1,&serde_json::to_vec(&json!({"world":[{"op":"set_environment","environment":{
                                "tonemap":"none","show_sky":false,"sun":{"direction":[-d.x,d.y,d.z],"intensity":intensity.unwrap_or(0.95),"color":[1.,1.,1.]}
                            }}]}))?)?;
                        }
                        other if actors::Actors::handles(&other) => self.actors.apply(other, &self.script, &self.files, &mut self.assets, op, now)?,

                        other => bail!("unsupported COM3D2 ADV effect {other:?}"),
                    }
                }
                if let KagStep::SceneCall(next)=step {
                    match next.as_str() {
                        "SceneADV" => {
                            self.script.enabled = true;
                            self.script.exec();
                        }
                        "SceneTitle" => self.enter(Scene::Title, op, host, client, now)?,
                        "SceneStartDaily" => self.enter(Scene::StartDaily, op, host, client, now)?,
                        "SceneEdit" => self.enter(Scene::StockEditor, op, host, client, now)?,
                        "SceneUserEdit" => self.enter(Scene::Editor, op, host, client, now)?,
                        "SceneNetorareCheck" => self.enter(Scene::Setup, op, host, client, now)?,
                        _ => bail!(
                            "COM3D2 scene {next} is not implemented (KAG {})",
                            self.script.position()
                        ),
                    }
                }
            }
            Scene::StartDaily => {
                if let Some(start) = self.leaving {
                    let age = now.saturating_sub(start);
                    if self.day_camera_fade.is_none() && age >= 250 {
                        self.day_camera_fade = Some(now);
                        self.sound.stop(host, client, Kind::Bgm, 1.)?;
                    }
                    if self.day_camera_fade.is_some_and(|since| now.saturating_sub(since) >= 1000) {
                        self.adv_camera = ([0., 1.5, 0.], 2., [180., 10.]);
                        let (target, distance, angle) = self.adv_camera;
                        let mut camera = crate::bg::orbit_camera(target, angle, distance, 35.);
                        camera["op"] = json!("set_camera");
                        camera["camera"]["near"] = json!(0.01);
                        host.submit_engine(client, 1, &serde_json::to_vec(&json!({"world":[camera]}))?)?;
                        self.resume_script(op, host, client, now)?;
                    }
                } else if elapsed >= 250
                    && input.buttons_down & (buttons::MOUSE_RIGHT | buttons::MOUSE_MIDDLE) == 0
                    && (input.buttons_down != 0 || released & buttons::MOUSE_LEFT != 0)
                {
                    self.leaving = Some(now);
                }

            }
            Scene::StockEditor => {
                if self.stock_editor.repeat_pose(now) {self.edit_pose(op,now)?;}
                let effects=self.actors.tick_motions(&self.files,op,now)?;
                self.edit_motion_effects(effects,op,now)?;
                if let Some(dialog)=&mut self.stock_editor.confirming {
                    match dialog.update(&self.widgets,input,released,now) {
                        Some(quit_dialog::Choice::Cancel)=>{self.widgets.truncate(dialog.first);self.stock_editor.confirming=None;}
                        Some(quit_dialog::Choice::Ok)=>bail!("COM3D2 SceneEdit profile finalization and portrait capture are not implemented"),
                        None=>{}
                    }
                }else if elapsed>=500 {
                    let mut tjs=self.script.tjs.lock().map_err(|_|anyhow::anyhow!("TJS lock"))?;
                    let state=&mut tjs.runtime.host_mut().state;
                    let id=state.active_maids[0].context("missing edit stock")?;
                    let previous=state.retail.stock_models[&id].clone();
                    let confirm=self.stock_editor.update(state,&mut self.widgets,input,released)?;
                    let changed=!std::sync::Arc::ptr_eq(&previous,&state.retail.stock_models[&id]);
                    drop(tjs);
                    if changed {self.actors.apply(crate::kag::AdvUi::AllProcProp{man:false,slot:0},&self.script,&self.files,&mut self.assets,op,now)?;}
                    if confirm {
                        let dialog=self.assets.scene(op,host,client,"SystemDialog")?;
                        self.stock_editor.confirming=Some(quit_dialog::QuitDialog::show(&mut self.widgets,dialog,now,"このメイドにしますか？")?);
                    }
                }
            }
            Scene::Editor => {
                if let Some(since) = self.leaving {
                    if now.saturating_sub(since) >= 1000 {
                        self.resume_script(op, host, client, now)?;
                    }
                } else {
                    let mut tjs = self
                        .script
                        .tjs
                        .lock()
                        .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                    if self.editor.update(
                        op,
                        host,
                        client,
                        &self.files,
                        &self.widgets,
                        &self.assets.editor_layout,
                        &mut tjs.runtime.host_mut().state,
                        input,
                        released,
                        elapsed >= 1000,
                        now,
                    )? {
                        self.leaving = Some(now);
                    }
                }
            }
            Scene::Setup => {
                if let Some(since) = self.leaving {
                    if now.saturating_sub(since) >= 1000 {
                        self.camera_control = true;
                        self.resume_script(op, host, client, now)?;
                    }
                } else if elapsed >= 1000 {
                    let hit = self.widgets.iter().position(|w| {
                        (w.toggle.is_some()
                            || (w.button.is_some() && w.button == self.assets.check_ok))
                            && w.collider.is_some_and(|[x, y, w, h]| {
                                let (px, py) = (input.pointer_x as f32, input.pointer_y as f32);
                                px >= x && px < x + w && py >= y && py < y + h
                            })
                    });
                    if input.buttons_pressed & buttons::MOUSE_LEFT != 0 {
                        self.pressed = hit;
                    }
                    if released & buttons::MOUSE_LEFT != 0 && self.pressed.take() == hit {
                        if let Some(index) = hit {
                            if let Some(toggle) = self.widgets[index].toggle {
                                for (i, widget) in self.widgets.iter_mut().enumerate() {
                                    if let Some(t) =
                                        widget.toggle.as_mut().filter(|t| t.group == toggle.group)
                                    {
                                        t.selected = i == index;
                                    }
                                }
                            } else {
                                let selected = |group: &str| -> Result<bool> {
                                    let suffix = format!("/{group}/Toggle_On");
                                    self.widgets
                                        .iter()
                                        .find(|w| w.path.ends_with(&suffix))
                                        .and_then(|w| w.toggle)
                                        .map(|t| t.selected)
                                        .with_context(|| format!("missing {group} toggle"))
                                };
                                let preferences = Preferences {
                                    lock_ntr_play: selected("Toggle_LockNTRPlay")?,
                                    lock_user_draft_maid: !selected("Toggle_LockUserDraftMaid")?,
                                    is_old_player: selected("Toggle_IsComPlayer")?,
                                };
                                {
                                    let mut tjs = self
                                        .script
                                        .tjs
                                        .lock()
                                        .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                                    let flags =
                                        &mut tjs.runtime.host_mut().state.player.generic_flag;
                                    flags.insert(
                                        "__lockNTRPlay".into(),
                                        i32::from(preferences.lock_ntr_play),
                                    );
                                    flags.insert(
                                        "__lockUserDraftMaid".into(),
                                        i32::from(preferences.lock_user_draft_maid),
                                    );
                                    flags.insert(
                                        "__isComPlayer".into(),
                                        i32::from(preferences.is_old_player),
                                    );
                                }
                                self.sound.stop(host, client, Kind::Bgm, 1.)?;
                                self.leaving = Some(now);
                            }
                        }
                    }
                }
            }
            Scene::Title => {
                if let Some(dialog) = &mut self.quit_dialog {
                    match dialog.update(&self.widgets, input, released, now) {
                        Some(quit_dialog::Choice::Ok) => self.quit_confirmed = true,
                        Some(quit_dialog::Choice::Cancel) => {
                            self.widgets.truncate(dialog.first);
                            self.quit_dialog = None;
                        }
                        None => {}
                    }
                } else if let Some(since) = self.leaving {
                    if now.saturating_sub(since) >= 3000 {
                        self.resume_script(op, host, client, now)?;
                    }
                } else if elapsed >= 3000 {
                    let p = [input.pointer_x, input.pointer_y];
                    let current = self.widgets.iter().find_map(|w| {
                        let index = Self::button(w)?;
                        if !w.path.ends_with(BUTTONS[index]) {
                            return None;
                        }
                        let [x, y, width, height] = Self::rect(self.scene, w, elapsed);
                        ((p[0] as f32) >= x
                            && (p[0] as f32) < x + width
                            && (p[1] as f32) >= y
                            && (p[1] as f32) < y + height)
                            .then_some(index)
                    });
                    if self.pointer != Some(p) {
                        if let Some(i) = current {
                            self.focus = i;
                        }
                        self.pointer = Some(p);
                    }
                    if input.buttons_pressed & buttons::DOWN != 0 {
                        self.focus = (self.focus + 1) % BUTTONS.len();
                    }
                    if input.buttons_pressed & buttons::UP != 0 {
                        self.focus = (self.focus + BUTTONS.len() - 1) % BUTTONS.len();
                    }
                    if input.buttons_pressed & buttons::MOUSE_LEFT != 0 {
                        self.pressed = current;
                    }
                    let click = released & buttons::MOUSE_LEFT != 0
                        && self.pressed.take() == current
                        && current.is_some();
                    if click || (input.buttons_pressed & buttons::ENTER) != 0 {
                        if click {
                            self.focus = current.unwrap();
                        }
                        let name = BUTTONS[self.focus];
                        if name == "Start" {
                            self.leaving = Some(now);
                        } else if name == "Shop" {
                            // CMSystem.ShopURL for the standalone, unlinked installation profile.
                            self.shop_requests.push(host.begin_desktop_request(client, DesktopRequest::OpenUrl {
                                url: "https://com3d2-shop.s-court.me/?ctv=1&cmd=0".into(),
                            })?);
                        } else if name == "End" {
                            let widgets = self.assets.scene(op, host, client, "SystemDialog")?;
                            self.quit_dialog = Some(quit_dialog::QuitDialog::show(&mut self.widgets, widgets, now,"終了しますか？")?);
                        } else {
                            bail!("COM3D2 TitleCtrl.On{name} is not implemented");
                        }
                    }
                }
            }
        }
        let elapsed = now.saturating_sub(self.entered);

        let fade_in = if self.scene == Scene::Title {
            3000.
        } else {
            1000.
        };
        let brightness = self
            .leaving
            .map_or((elapsed as f32 / fade_in).min(1.), |since| {
                (1. - now.saturating_sub(since) as f32 / fade_in).clamp(0., 1.)
            });
        let brightness = if self.scene == Scene::Script {
            self.adv_cover.map_or(brightness, |f| 1. - f.value(now))
        } else if self.scene == Scene::StartDaily {
            self.day_camera_fade.map_or((elapsed as f32 / 1000.).min(1.), |start| (1. - now.saturating_sub(start) as f32 / 1000.).clamp(0., 1.))
        } else {
            brightness
        };
        if self.scene == Scene::Script
            && let Some(t) = &self.camera_move
        {
            let ratio = if t.duration == 0 {
                1.
            } else {
                crate::adapter::ease(
                    &t.ease,
                    now.saturating_sub(t.start) as f32 / t.duration as f32,
                )
            };
            let lerp = |a: f32, b: f32| a + (b - a) * ratio;
            self.adv_camera = (
                std::array::from_fn(|i| lerp(t.from.0[i], t.to.0[i])),
                lerp(t.from.1, t.to.1),
                std::array::from_fn(|i| lerp(t.from.2[i], t.to.2[i])),
            );
            let (pos, dist, angles) = self.adv_camera;
            let mut cmd = crate::bg::orbit_camera(pos, angles, dist, 35.);
            cmd["op"] = json!("set_camera");
            host.submit_engine(client, 1, &serde_json::to_vec(&json!({"world":[cmd]}))?)?;
            if now >= t.start + t.duration {
                self.camera_move = None;
            }
        }
        if matches!(self.scene,Scene::Script|Scene::StockEditor) { self.actors.sync(host, client, &self.files, op, now, &mut self.adv_camera)?; }
        let mut banner = None;
        if self.scene == Scene::Script {
            let request = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?
                .runtime
                .host_mut()
                .state
                .retail
                .title_request
                .take();
            if let Some(title) = request {
                self.title_banner = Some((title, None));
            }
            if let Some((text, start)) = &mut self.title_banner {
                let label = self
                    .widgets
                    .iter_mut()
                    .find(|w| w.path.ends_with("/TitleLabel"))
                    .context("missing SceneADV title label")?;
                let x = label.rect[0];
                if let Some(width) = label
                    .label
                    .as_mut()
                    .context("title UILabel")?
                    .title_width(host, client, text)?
                {
                    label.rect[2] = width;
                    let bg_width = x + width + 100.;
                    let (shift, alpha) = title_frame(
                        now.saturating_sub(*start.get_or_insert(now)),
                        self.assets.title_display_ms,
                        bg_width,
                    );
                    banner = Some((bg_width, shift, alpha));
                }
            }
        }
        self.trophies.update(now,brightness<1.,&mut self.assets,op,host,client,&self.script,&self.files,&mut self.sound)?;
        let scene_count=self.widgets.len();
        let trophy_transform = self.trophies.transform(now);
        let mut layers = Vec::new();
        let sprites: std::collections::HashMap<_, _> = self
            .widgets
            .iter()
            .filter_map(|w| w.toggle.map(|t| (t.active_sprite, t.selected)))
            .collect();
        for (i, w) in self.widgets.iter_mut().chain(&mut self.trophies.widgets).enumerate() {
            let overlay = i >= scene_count;
            let mut rect = if overlay { w.rect } else { Self::rect(self.scene, w, elapsed) };
            let mut clip = if self.scene==Scene::StockEditor {self.stock_editor.clip(&w.path)} else {None};
            let mut editor_visible = true;
            if !overlay && self.scene == Scene::Editor {
                self.editor.decorate(w);
                (rect, clip, editor_visible) = self.editor.geometry(w, &self.assets.editor_layout);
            }
            let (shift, opacity) =
                if overlay {trophy_transform} else if let Some(choices)=&self.choices && i>=choices.first {
                    let (dy,bounds,show,alpha)=choices.draw(i,&w.path,now);
                    rect[1]+=dy;
                    clip=Some(bounds);
                    editor_visible=show;
                    (0.,alpha)
                } else if self.scene == Scene::Script && w.path.contains("/MessageWindowPanel/") {
                    let Some((name, text)) = &self.dialogue else {
                        continue;
                    };
                    if !self.message_open || (name.is_empty() && w.path.contains("/SpeakerName")) {
                        continue;
                    }
                    if let Some(label) = w.label.as_mut() {
                        label.set_text(if w.path.ends_with("/Name") {
                            name
                        } else {
                            text
                        });
                    }
                    (0., 255)
                } else if self.scene == Scene::Script {
                    let Some((width, shift, opacity)) = banner else {
                        continue;
                    };
                    if w.path.ends_with("/BG") {
                        rect[2] = width;
                    }
                    (shift, opacity)
                } else {
                    (
                        0.,
                        if self.scene == Scene::StartDaily {
                            let alpha = self.leaving.map_or(elapsed as f32 / 250., |start| 1. - now.saturating_sub(start) as f32 / 250.);
                            (255. * alpha.clamp(0., 1.)).round() as u8
                        } else if let Some(dialog) = self.quit_dialog.as_ref().or(self.stock_editor.confirming.as_ref()) && i >= dialog.first {
                            dialog.opacity(now)
                        } else if w.path.starts_with("/Canvas Tutorial Panel/") {
                            self.editor.help_alpha(elapsed, now)
                        } else {
                            255
                        },
                    )
                };
            let visible = if !overlay && w.path.ends_with("/SelectCursor") && (self.scene==Scene::Title || self.stock_editor.confirming.is_some()) {
                if let Some(dialog) = self.quit_dialog.as_ref().or(self.stock_editor.confirming.as_ref()) { dialog.cursor(&w.path) } else { Self::button(w) == Some(self.focus) }
            } else {
                overlay || (editor_visible && sprites.get(&w.component).copied().unwrap_or(true))
            };
            if let Some(label) = w.label.as_mut() {
                for (pos, paint) in label.layers(host, client, w.rect, w.tint)? {
                    let id = layers.len() + 1;
                    let mut pos = *pos;
                    pos[0] += shift + rect[0] - w.rect[0];
                    pos[1] += rect[1] - w.rect[1];
                    layers.push(json!({"op":"put","layer":{"id":id,"rect":pos,"viewport_clip":clip,"opacity":opacity,"paint":paint,"visible":visible,"order":[w.depth,id]}}));
                }
            } else {
                let id = layers.len() + 1;
                rect[0] += shift;
                let paint = if w.asset.is_empty() {
                    json!({"kind":"solid","color":w.tint})
                } else {
                    json!({"kind":"image","asset":w.asset,"tint":w.tint})
                };
                if let Some(slices) = w.slices {
                    slices.draw(rect, w.uv, |piece, uv, scale| {
                        let id = layers.len() + 1;
                        layers.push(json!({"op":"put","layer":{"id":id,"rect":piece,"uv":uv,
                            "transform":{"scale_x":scale[0],"scale_y":scale[1]},
                            "viewport_clip":clip,"opacity":opacity,"visible":visible,"paint":paint,"order":[w.depth,id]}}));
                    });
                } else {
                    layers.push(json!({"op":"put","layer":{"id":id,"rect":rect,"viewport_clip":clip,"opacity":opacity,"uv":w.uv,"visible":visible,"paint":paint,"order":[w.depth,id]}}));
                }
            }
        }
        let count = layers.len();
        for id in count + 1..=self.previous_layers {
            layers.push(json!({"op":"remove","id":id}));
        }
        self.previous_layers = count;
        if let Some(fade) = self.world_cover {
            let opacity = if self.scene == Scene::Script { (255. * fade.value(now)).round() as u8 } else { 0 };
            layers.push(json!({"op":"put","layer":{"id":65534,"rect":[0,0,1920,1080],"paint":{"kind":"solid","color":[0,0,0,255]},"opacity":opacity,"order":[-100000,0]}}));
        }
        layers.push(json!({"op":"put","layer":{"id":65535,"rect":[0,0,1920,1080],"paint":{"kind":"solid","color":[0,0,0,255]},"opacity":((1.-brightness)*255.) as u8,"order":[100000,0]}}));
        host.submit_engine(client, 1, &serde_json::to_vec(&json!({"layers":layers}))?)?;
        Ok(())
    }
}

struct Vm {
    client: ClientId,
    config: Config,
    game: Option<Game>,
}
impl Vm {
    fn advance(&mut self, host: &mut dyn HostServices, input: InputFrame) -> Result<()> {
        let mut op = Opener {
            storage: host.storage(self.client)?,
            dir: self.config.data_dir.clone(),
        };
        if self.game.is_none() {
            let files = std::sync::Arc::new(GameFiles::index(
                op.storage.as_ref(),
                &self.config.archives,
            )?);
            let mut state = crate::tjs::TjsState::default();
            state
                .retail
                .attach(files.clone(), op.storage.clone(), &op.dir)?;
            let script = AdvKag::for_profile(
                &files,
                op.storage.as_ref(),
                crate::tjs::ScriptProfile::Com3d2,
                state,
            )?;
            let assets = Assets::load(&mut op).context("load standalone Unity assets")?;
            let now = host.now_ms();
            let mut g = Game {
                adv_camera: ([0., 1.6, 0.], 3., [180., 0.]),
                camera_move: None,
                adv_cover: None,
                world_cover: None,
                dialogue: None,
                editor: Default::default(),
                actors: Default::default(),
                stock_editor: Default::default(),
                assets,
                files,
                script,
                sound: Sound::new(),
                scene: Scene::Logo,
                widgets: vec![],
                title_banner: None,
                message_open: false,
                camera_control: true,
                previous_layers: 0,
                club: None,
                club_meshes: 0,
                entered: now,
                leaving: None,
                down: 0,
                pointer: None,
                pressed: None,
                focus: 0,
                quit_dialog: None,
                choices: None,
                quit_confirmed: false,
                day_camera_fade: None,
                trophies: Default::default(),
                shop_requests: Vec::new(),
            };
            g.enter(Scene::Logo, &mut op, host, self.client, now)?;
            self.game = Some(g);
        }
        let game = self.game.as_mut().unwrap();
        game.update(host, self.client, &mut op, input).with_context(|| format!("update {}", game.scene.name()))
    }
}
impl VmAdapter for Vm {
    fn engine_id(&self) -> &'static str {
        "com3d2"
    }
    fn snapshot_schema(&self) -> u32 {
        1
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported("COM3D2 snapshot"))
    }
    fn restore(&mut self, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported("COM3D2 snapshot"))
    }
    fn step(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
        _: u32,
    ) -> crysirror_core::Result<VmYield> {
        self.advance(host, input)
            .map_err(|e| Error::Io(format!("{e:#}")))?;
        Ok(if self.game.as_ref().is_some_and(|g|g.quit_confirmed) { VmYield::Halted } else { VmYield::WaitUntil(host.now_ms() + 1) })
    }
    fn diagnostics(&self) -> Vec<u8> {
        let state = self.game.as_ref().map(|game| {
            let preferences = game.script.tjs.lock().ok().and_then(|tjs| {
                let player=&tjs.runtime.host().state.player;
                player.generic_flag.contains("__isComPlayer").then(||Preferences {
                    lock_ntr_play: player.flag("__lockNTRPlay")!=0,
                    lock_user_draft_maid: player.flag("__lockUserDraftMaid")!=0,
                    is_old_player: player.flag("__isComPlayer")!=0,
                })
            });
            let day = game.script.tjs.lock().ok().map(|vm| {
                let state = &vm.runtime.host().state;
                json!({"number":state.player.days,"facility_record":state.retail.peak_facilities,
                    "night_work_states":state.player.night_works_state.len()})
            });
            let selected:Vec<_>=game.widgets.iter().filter(|w|w.toggle.is_some_and(|t|t.selected)).map(|w|&w.path).collect();
            json!({"scene":game.scene.name(),"day":day,"quit_dialog":game.quit_dialog.is_some(),"choices":game.choices.as_ref().map(|c|c.labels()),"widgets":game.widgets.len(),"stock_editor":if game.scene==Scene::StockEditor {game.script.tjs.lock().ok().map(|t|game.stock_editor.diagnostics(&t.runtime.host().state))}else{None},"player_editor":game.editor.diagnostics(),"dialogue":game.dialogue,"npc_slots":game.actors.slots.iter().map(|((man,slot),i)|json!([man,slot,i])).collect::<Vec<_>>(),"club_meshes":game.club_meshes,
                "preferences":preferences,"selected":selected,"focus":BUTTONS[game.focus],
                "script":game.script.position(),"bgm":game.sound.current_bgm})
        });
        serde_json::to_vec(&state).unwrap_or_default()
    }
}
pub(crate) struct Factory;
#[cfg(feature = "runtime")]
impl crysirror_runtime::runtime::VmFactory for Factory {
    fn create(
        &self,
        client: ClientId,
        spec: &crysirror_runtime::model::ClientSpec,
    ) -> Result<Box<dyn VmAdapter>> {
        ensure!(spec.program.is_empty(), "COM3D2 uses native scripts");
        let config: Config = serde_json::from_value(spec.config.clone())?;
        config.validate()?;
        Ok(Box::new(Vm {
            client,
            config,
            game: None,
        }))
    }
}
