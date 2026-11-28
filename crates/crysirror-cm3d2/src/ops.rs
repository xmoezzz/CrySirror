// SPDX-License-Identifier: MPL-2.0
//! Execution of the KAG presentation requests beyond the message window and
//! camera (`adapter.rs`): character motion, items, masks, face, voice, the
//! choice buttons and prefabs, as their `CharacterMgr`/`Maid`/`TBody`/
//! `MessageWindowMgr`/`BgMgr` methods do.
use crate::{
    adapter::{Game, Opener},
    kag::AdvUi,
};
use anyhow::{Context, Result};
use serde_json::json;

/// `SelectorViewer/Contents/SelectButtonParent` under the message window.
const SELECT_PARENT: &str = "__GameMain__/SystemUI Root/MessageWindowPanel/SelectorViewer/Contents/SelectButtonParent";
const SELECTOR: &str = "__GameMain__/SystemUI Root/MessageWindowPanel/SelectorViewer";

/// An NGUI `TweenAlpha` of a `__GameMain__` node.
#[derive(Clone, Debug)]
pub struct Tween {
    pub node: usize,
    pub from: f32,
    pub to: f32,
    pub start: u64,
    pub ms: u64,
    /// `onFinished` handler (`Class.Method`).
    pub finished: Option<String>,
}

/// A `SelectButtonCtrl` button: node, text and label.
#[derive(Clone, Debug)]
pub struct ChoiceButton {
    pub node: usize,
    pub text: String,
    pub label: String,
}

impl Game {
    pub(crate) fn chara(&mut self, man: bool, slot: usize) -> Option<&mut crate::chara::Maid> {
        if man { self.man(slot) } else { self.maid(slot) }
    }

    /// `.anm` clip length (seconds), cached.
    pub fn anm_duration(&mut self, opener: &mut Opener, file: &str) -> Result<f32> {
        if let Some(d) = self.anm_durations.get(file) {
            return Ok(*d);
        }
        let bytes = self.files.read(opener.storage.as_ref(), file)?;
        let anim = crysirror_asset::interchange::cm3d2::Animation::parse(&bytes).with_context(|| format!("motion {file}"))?;
        let d = anim
            .tracks
            .iter()
            .flat_map(|t| t.curves.iter().flatten())
            .flat_map(|c| c.keys.iter().map(|k| k[0]))
            .fold(0f32, f32::max);
        self.anm_durations.insert(file.to_owned(), d);
        Ok(d)
    }

    /// `ScriptManager.StopMotionScript`: every motion KAG stops.
    pub fn stop_motion_scripts(&mut self) {
        for kag in self.motion_kags.values_mut() {
            kag.clear_motion();
            kag.enabled = false;
        }
    }

    /// Requests not handled by `Game::adv_ui`.
    pub fn adv_ui_ext(&mut self, op: AdvUi, opener: &mut Opener) -> Result<()> {
        let now = self.now;
        match op {
            AdvUi::MotionBlend(on) => self.motion_blend = on,
            AdvUi::StopMotionScripts => self.stop_motion_scripts(),
            AdvUi::Motion { man, slot, file, looping, blend_s, queue } => {
                // BaseKagManager.PlayMaidMotion: no blend while disabled.
                let fade = if self.motion_blend { blend_s } else { 0. };
                if !self.files.exists(&file) {
                    anyhow::bail!("モーション {file} が見つかりません。");
                }
                let duration = self.anm_duration(opener, &file)?;
                if let Some(c) = self.chara(man, slot) {
                    c.cross_fade(&file, looping, queue, fade, now, duration);
                }
            }
            AdvUi::MotionStop { man, slot } => {
                if let Some(c) = self.chara(man, slot) {
                    c.stop_anime();
                }
            }
            AdvUi::MotionLayerStop { man, slot, layers } => {
                if let Some(c) = self.chara(man, slot) {
                    for l in layers {
                        anyhow::ensure!(l >= 2, "モーションレイヤーの停止は2以上を指定して下さい。");
                        c.stop_anime_layer(l as u32);
                    }
                }
            }
            AdvUi::AutoTwist { man, slot, twists } => {
                if let Some(c) = self.chara(man, slot) {
                    for (k, on) in twists {
                        c.auto_twist.insert(k, on);
                    }
                }
            }
            AdvUi::HairPhysics { man, slot, mode } => {
                if let Some(c) = self.chara(man, slot) {
                    // SetModeHairR(Normal) for both accepted values.
                    let _ = mode;
                    c.hair_mode = "normal".into();
                }
            }
            AdvUi::BustMove { man, slot, left, right } => {
                if let Some(c) = self.chara(man, slot) {
                    if let Some(l) = left {
                        c.bust_move[0] = l;
                    }
                    if let Some(r) = right {
                        c.bust_move[1] = r;
                    }
                }
            }
            AdvUi::PhysicsHit { man, slot, height } => {
                if let Some(c) = self.chara(man, slot) {
                    c.hit_height = height;
                }
            }
            AdvUi::Ik { src, bone, target, offset } => {
                if let Some(c) = self.chara(src.0, src.1) {
                    match target {
                        Some((who, t)) => {
                            c.ik.insert(bone, (who, t, offset));
                        }
                        None => {
                            c.ik.remove(&bone);
                        }
                    }
                }
            }
            AdvUi::AllOffset { position, rotation } => {
                if let Some(p) = position {
                    self.all_offset = p;
                }
                if let Some(r) = rotation {
                    self.all_offset_rot = r;
                }
            }
            AdvUi::AllResetPos => {
                self.all_pos = [0.; 3];
                self.all_rot = [0.; 3];
                self.all_offset = [0.; 3];
                self.all_offset_rot = [0.; 3];
                for c in &mut self.characters {
                    c.position = [0.; 3];
                    c.rotation = [0.; 3];
                    c.offset_position = [0.; 3];
                    c.hit_height = 0.;
                }
            }
            AdvUi::CameraChase { man, slot, bone, offset, once, seconds } => {
                let index = if man { self.active_men.get(slot).copied().flatten() } else { self.active_maids.get(slot).copied().flatten() };
                self.camera_chase = index.map(|i| crate::adapter::Chase { chara: i, bone, offset, once, seconds, start: now, from: None });
            }
            AdvUi::CameraChaseStop => self.camera_chase = None,
            AdvUi::Voice { who, file, fade_s, looping } => {
                // A visible maid's AudioMan, else PlayDummyVoice: both are
                // voice channels of SoundMgr.
                let _ = who;
                self.sound_ops.push(crate::sound::SoundOp::Play { kind: crate::sound::Kind::Voice, file, fade_s, looping });
            }
            AdvUi::VoiceStop { .. } => self.sound_ops.push(crate::sound::SoundOp::Stop { kind: crate::sound::Kind::Voice, fade_s: 0. }),
            AdvUi::StopSound => {
                for kind in [crate::sound::Kind::Bgm, crate::sound::Kind::Env, crate::sound::Kind::Se, crate::sound::Kind::Voice, crate::sound::Kind::System] {
                    self.sound_ops.push(crate::sound::SoundOp::Stop { kind, fade_s: 0. });
                }
            }
            AdvUi::TalkVoice { file, maid, chara_name } => {
                // TagHitRet: VoiceStopAll, then the voice unless skipping.
                self.sound_ops.push(crate::sound::SoundOp::Stop { kind: crate::sound::Kind::Voice, fade_s: 0. });
                let _ = (maid, chara_name);
                if let Some(last) = self.backlog.last_mut() {
                    last.2 = file.clone();
                }
                if !self.skip_mode {
                    if let Some(v) = self.main.scene.find("__GameMain__/SystemUI Root/MessageWindowPanel/MessageViewer/Buttons/Voice") {
                        self.main.states.entry(v).or_default().enabled = Some(true);
                    }
                    self.sound_ops.push(crate::sound::SoundOp::Play { kind: crate::sound::Kind::Voice, file, fade_s: 0., looping: false });
                }
            }
            AdvUi::Sound(op) => self.sound_ops.push(op),
            AdvUi::ItemChange { man, slot, category, name } => {
                if let Some(c) = self.chara(man, slot) {
                    c.item_change_temp(&category, &name)?;
                }
            }
            AdvUi::ItemSet { man, slot, category, item, temp } => {
                if let Some(c) = self.chara(man, slot) {
                    c.set_prop_file(&category, &item, temp)?;
                }
            }
            AdvUi::ItemReset { man, slot, props } => {
                if let Some(c) = self.chara(man, slot) {
                    for p in props {
                        c.reset_prop(&p)?;
                    }
                }
            }
            AdvUi::AllProcProp { man, slot } => {
                let storage = opener.storage.clone();
                let files = self.files.clone();
                if let Some(c) = self.chara(man, slot) {
                    c.all_proc_prop(&files, storage.as_ref())?;
                    let warnings = std::mem::take(&mut c.warnings);
                    self.log.extend(warnings);
                }
            }
            AdvUi::ItemMask { man, slot, body_slot, visible } => {
                if let Some(c) = self.chara(man, slot) {
                    c.set_mask(&body_slot, visible)?;
                }
            }
            AdvUi::ItemMaskMode { man, slot, mode } => {
                if let Some(c) = self.chara(man, slot) {
                    c.set_mask_mode(&mode)?;
                }
            }
            AdvUi::Chinko { slot, on } => {
                if let Some(c) = self.man(slot) {
                    c.chinko_visible = on;
                }
            }
            AdvUi::FaceBlend { man, slot, name } => {
                if let Some(c) = self.chara(man, slot) {
                    c.face.blend(&name);
                }
            }
            AdvUi::OpenMouth { man, slot, open } => {
                if let Some(c) = self.chara(man, slot) {
                    c.face.mouth_open = open;
                }
            }
            AdvUi::Lipsync { man, slot, on } => {
                if let Some(c) = self.chara(man, slot) {
                    c.face.lipsync = on;
                }
            }
            AdvUi::Kuchipaku { man, slot, pattern } => {
                use base64::Engine;
                let pattern = match pattern {
                    Some((offset, text)) => Some((offset, base64::engine::general_purpose::STANDARD.decode(text.trim()).context("kuchipaku pattern")?)),
                    None => None,
                };
                if let Some(c) = self.chara(man, slot) {
                    c.face.kuchipaku = pattern;
                }
            }
            AdvUi::Eye { man, slot, look, blend_s } => {
                if let Some(c) = self.chara(man, slot) {
                    c.eye = (look, blend_s);
                }
            }
            AdvUi::MaidOffsetCategory { man, slot, category, offset } => {
                if let Some(c) = self.chara(man, slot) {
                    // SetPosOffsetFromUdeScl{X,Y,Z}: (prop − 50) · (v / 50).
                    let v = c.value(&category) as f32;
                    for (k, o) in offset.iter().enumerate() {
                        if let Some(o) = o {
                            c.offset_position[k] = (v - 50.) * (o / 50.);
                        }
                    }
                }
            }
            AdvUi::AddPrefabBg { src, name, dest, position, rotation } => self.add_prefab_bg(opener, &src, &name, &dest, position, rotation)?,
            AdvUi::DelPrefabBg(name) => {
                let keep: Vec<_> = std::mem::take(&mut self.bg_prefabs);
                for (n, id) in keep {
                    if name.as_ref().is_none_or(|x| *x == n) {
                        self.world.push(json!({"op": "despawn", "id": id}));
                    } else {
                        self.bg_prefabs.push((n, id));
                    }
                }
            }
            AdvUi::MovePrefabToChara { man, slot, name, bone, offset, seconds } => {
                // BgMgr.CoWaitMovePrefabToChara: reparent after `seconds`.
                let _ = seconds;
                let (src, pos) = (name.clone(), offset);
                if let Some(i) = self.bg_prefabs.iter().position(|(n, _)| *n == name) {
                    let (_, id) = self.bg_prefabs.remove(i);
                    self.world.push(json!({"op": "despawn", "id": id}));
                }
                if let Some(c) = self.chara(man, slot) {
                    c.prefabs.insert(name, (src, bone, pos, [0.; 3]));
                }
            }
            AdvUi::AddPrefabChara { man, slot, src, name, bone, position, rotation } => {
                anyhow::ensure!(
                    self.unity.resource_of(opener, &format!("Prefab/{src}"), crysirror_unity::class_id::GAME_OBJECT)?.is_some(),
                    "人物にプレハブ追加で、プレハブが見つかりません。Prefab/{src}"
                );
                if let Some(c) = self.chara(man, slot) {
                    c.prefabs.insert(name, (src, bone, position, rotation));
                }
            }
            AdvUi::DelPrefabChara { man, slot, name } => {
                if let Some(c) = self.chara(man, slot) {
                    match name {
                        Some(n) => {
                            c.prefabs.remove(&n);
                        }
                        None => c.prefabs.clear(),
                    }
                }
            }
            AdvUi::AnimObjChara { man, slot, name, anim, play, looping } => {
                if let Some(c) = self.chara(man, slot) {
                    anyhow::ensure!(c.prefabs.contains_key(&name), "人物に追加した {name} は見つかりませんでした。");
                }
                self.log.push(format!("animobjchara {man}/{slot} {name} {anim} play={play} loop={looping}"));
            }
            AdvUi::TexMulAdd { man, slot, body_slot, material, property, layer, file, at } => {
                if let Some(path) = file.strip_prefix("res:") {
                    let key = format!("{file}.tex");
                    if !self.res_textures.contains_key(&key) {
                        let tk = self
                            .unity
                            .resource_of(opener, &format!("Texture/{path}"), crysirror_unity::class_id::TEXTURE_2D)?
                            .with_context(|| format!("Resourcesからテクスチャ Texture/{path} はみつかりません。"))?;
                        let (_, bytes) = self.unity.bytes(opener, &tk)?;
                        let img = crysirror_unity::texture::decode(&crysirror_unity::classes::Texture2D::read(&bytes)?)?;
                        self.res_textures.insert(key, crysirror_image::encode_png(&img)?);
                    }
                }
                if let Some(c) = self.chara(man, slot) {
                    c.tex_mul_add(&body_slot, material, &property, layer, &file, at);
                    if property == "_MainTex" {
                        c.tex_mul_add(&body_slot, material, "_ShadowTex", layer, &file, at);
                    }
                }
            }
            AdvUi::TexMulDel { man, slot, body_slot, material, property } => {
                if let Some(c) = self.chara(man, slot) {
                    c.tex_mul_del(&body_slot, material, &property);
                    if property == "_MainTex" {
                        c.tex_mul_del(&body_slot, material, "_ShadowTex");
                    }
                    c.visible = true;
                }
            }
            AdvUi::StartDay => self.on_start_day()?,
            AdvUi::EndDay => self.on_end_day()?,
            AdvUi::Choices(list) => {
                self.log.push(format!("choices {:?} at {}", list, self.adv.position()));
                self.create_select_buttons(opener, list)?
            }
            AdvUi::StartMvpEffect => {
                // KagScript.CallTag: without SceneSingleEffect the tag has
                // no callback.
                if self.active == crate::scenes::Active::SingleEffect {
                    crate::wf::set_active(self, "UI Root/Main/MVPEffect", true)?;
                    // SATD-cm3d2-rendering: the MVP sprite's Animator clip and
                    // the PKamihubuki particle system are not played.
                    if self.bg.as_ref().is_some_and(|b| b.0 == "Salon") {
                        self.add_prefab_bg(opener, "Particle/PKamihubuki", "PKamihubuki", "", [0., 3.5, 0.], [0.; 3])?;
                    }
                }
            }
            AdvUi::YotogiCall { name, label, edit_label, .. } => match &mut self.active {
                crate::scenes::Active::Yotogi(y) => y.call(name, label, edit_label),
                _ => anyhow::bail!("@yotogicall outside SceneYotogi"),
            },
            AdvUi::PlayBgmRoom { fade_s } => {
                // Yotogi.GetStageData(StagePrefab).bgm_file.
                let prefab = self.adv.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?.runtime.host().state.stage_prefab.clone();
                let data = self.adv.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?.runtime.host().state.data()?;
                let stage = data.yotogi.stage_by_prefab(&prefab).with_context(|| format!("@playbgmroom: no stage of {prefab}"))?;
                self.sound_ops.push(crate::sound::SoundOp::Play { kind: crate::sound::Kind::Bgm, file: stage.bgm_file.clone(), fade_s, looping: true });
            }
            AdvUi::ControlMask => {
                if let crate::scenes::Active::Yotogi(mut y) = std::mem::replace(&mut self.active, crate::scenes::Active::Adv) {
                    let r = y.control_mask(self);
                    self.active = crate::scenes::Active::Yotogi(y);
                    r?;
                }
            }
            AdvUi::RepeatVoice(file) => {
                if let crate::scenes::Active::Yotogi(y) = &mut self.active {
                    y.set_repeat_voice(file);
                }
            }
            AdvUi::CharaActivateLeader { slot } => {
                let leader = {
                    let tjs = self.adv.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
                    let st = &tjs.runtime.host().state;
                    st.stock.iter().copied().find(|i| st.maids.get(i).is_some_and(|m| m.leader))
                };
                if let Some(i) = leader {
                    *self.active_maids.get_mut(slot).context("maid slot")? = Some(i);
                    self.characters[i].visible = true;
                    self.adv_ui(AdvUi::AllProcProp { man: false, slot }, opener)?;
                }
            }
            other => anyhow::bail!("unhandled KAG request {other:?}"),
        }
        Ok(())
    }

    /// `BgMgr.AddPrefabToBg`: `Resources.Load("Prefab/" + src)` under the
    /// background (or its `dest` child).
    fn add_prefab_bg(&mut self, opener: &mut Opener, src: &str, name: &str, dest: &str, position: [f32; 3], rotation: [f32; 3]) -> Result<()> {
        let Some((_, bg)) = self.bg.clone() else {
            self.log.push(format!("BGが未だロードされていないのでプレハブは付けられませんでした。{src}"));
            return Ok(());
        };
        let go = self
            .unity
            .resource_of(opener, &format!("Prefab/{src}"), crysirror_unity::class_id::GAME_OBJECT)?
            .with_context(|| format!("Prefab/の下に指定のプレハブが見つかりませんでした。{src}"))?;
        let scene = self.unity.prefab(opener, &go)?;
        let Some(glb) = crate::bg::build(&mut self.unity, opener, &scene, scene.roots[0])? else { return Ok(()) };
        self.next_model += 1;
        self.next_entity += 1;
        let id = 0x100 + self.next_entity;
        self.pending_models.push((self.next_model, format!("bg:{id}"), glb));
        // Parent: the background root, or its `dest` child (static).
        let parent = match (&self.bg_scene, dest.is_empty()) {
            (Some(scene), false) => {
                let root = scene.roots[0];
                let node = scene.descendant(root, dest).with_context(|| format!("BG child {dest}"))?;
                scene.global(root).inverse() * scene.global(node)
            }
            _ => glam::Mat4::IDENTITY,
        };
        let local = glam::Mat4::from_rotation_translation(crate::chara::unity_euler(rotation), glam::Vec3::from_array(position));
        let (_, q, p) = (parent * local).to_scale_rotation_translation();
        let m = glam::Quat::from_xyzw(q.x, -q.y, -q.z, q.w);
        let (x, y, z) = m.to_euler(glam::EulerRot::XYZ);
        let transform = json!({"translation": [-p.x, p.y, p.z], "rotation_degrees": [x.to_degrees(), y.to_degrees(), z.to_degrees()], "scale": [1., 1., 1.]});
        self.pending_parents.push((id, bg, None, transform));
        self.bg_prefabs.push((name.to_owned(), id));
        Ok(())
    }

    /// `MessageWindowCtrl.CreateSelectButtons` → `SelectButtonCtrl`: one
    /// `SelectButton` prefab per choice (`selectButton_N`, the `Option`
    /// label), `UIGrid.Reposition`, the scroll arrows shown.
    fn create_select_buttons(&mut self, opener: &mut Opener, list: Vec<(String, String)>) -> Result<()> {
        self.clear_select_buttons();
        let go = self
            .unity
            .resource_of(opener, "SystemUI/MessageWindow/Main/Prefabs/SelectButton", crysirror_unity::class_id::GAME_OBJECT)?
            .context("ロードされるプレハブが見つかりませんでした。パス=SystemUI/MessageWindow/Main/Prefabs/SelectButton")?;
        let prefab = self.unity.prefab(opener, &go)?;
        let parent = self.main.scene.find(SELECT_PARENT).context(SELECT_PARENT)?;
        for (i, (text, label)) in list.into_iter().enumerate() {
            let node = self.main.scene.instantiate(&prefab, prefab.roots[0], Some(parent));
            self.main.scene.nodes[node].name = format!("selectButton_{i}");
            self.main.scene.nodes[node].position = [0.; 3];
            self.main.scene.nodes[node].scale = [1.; 3];
            let text = self.replace_chara_name(&text);
            if let Some(opt) = self.main.scene.child(node, "Option") {
                self.main.states.entry(opt).or_default().text = Some(text.clone());
            }
            self.main.states.entry(node).or_default().click = Some("SelectButtonCtrl.ClickSelectButton".into());
            self.choices.push(crate::ops::ChoiceButton { node, text, label });
        }
        for (node, p) in crate::ngui::grid_positions(&self.main.scene, &self.main.states, parent) {
            self.main.states.entry(node).or_default().position = Some(p);
        }
        for arrow in ["Scroll Bar/Down", "Scroll Bar/Up"] {
            if let Some(n) = self.main.scene.find(&format!("{SELECTOR}/{arrow}")) {
                self.main.states.entry(n).or_default().active = Some(true);
            }
        }
        Ok(())
    }

    /// `SelectButtonCtrl.ClearExistSelectButton`.
    pub fn clear_select_buttons(&mut self) {
        if let Some(parent) = self.main.scene.find(SELECT_PARENT) {
            for c in std::mem::take(&mut self.main.scene.nodes[parent].children) {
                self.main.states.remove(&c);
            }
        }
        self.choices.clear();
    }

    /// `SelectButtonCtrl.ClickSelectButton` / `OnFinished`.
    pub fn choice_event(&mut self, opener: &mut Opener, node: usize, handler: &str) -> Result<bool> {
        match handler {
            "SelectButtonCtrl.ClickSelectButton" => {
                let skip = self.skip_mode;
                for b in self.choices.clone() {
                    let (ms, finished) = if b.node == node {
                        self.choice_pending = Some((b.text.clone(), b.label.clone()));
                        (if skip { 10 } else { 1000 }, Some("SelectButtonCtrl.OnFinished".to_owned()))
                    } else {
                        (if skip { 10 } else { 300 }, None)
                    };
                    self.tweens.push(Tween { node: b.node, from: 1., to: 0., start: self.now, ms, finished });
                    self.main.states.entry(b.node).or_default().collider = Some(false);
                }
                Ok(true)
            }
            "SelectButtonCtrl.OnFinished" => {
                if let Some(n) = self.main.scene.find(&format!("{SELECTOR}/DragMat")) {
                    self.main.states.entry(n).or_default().active = Some(false);
                }
                self.clear_select_buttons();
                if let Some((text, label)) = self.choice_pending.take() {
                    // Tag_ChoicesShow callback: backlog, GoToLabel, Exec.
                    self.backlog.push((String::new(), format!("=> {text}"), String::new()));
                    let storage = opener.storage.clone();
                    self.adv.choose(&self.files, storage.as_ref(), &label)?;
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// `TweenAlpha` progress of this frame; finished tweens fire their
    /// handlers as NGUI events.
    pub fn update_tweens(&mut self) {
        let now = self.now;
        let mut done = Vec::new();
        for (i, t) in self.tweens.iter().enumerate() {
            let f = if t.ms == 0 { 1. } else { (now.saturating_sub(t.start) as f32 / t.ms as f32).min(1.) };
            self.main.states.entry(t.node).or_default().alpha = Some(t.from + (t.to - t.from) * f);
            if f >= 1. {
                done.push(i);
            }
        }
        for i in done.into_iter().rev() {
            let t = self.tweens.remove(i);
            if let Some(h) = t.finished {
                self.events.push((0, t.node, h));
            }
        }
    }

    /// `GameMain.OnStartDay`: a leader among the stock maids (else the
    /// first active maid 0..=4, else the first stock maid), one more day and
    /// phase day, the ranking day counter.
    fn on_start_day(&mut self) -> Result<()> {
        let mut tjs = self.adv.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let state = &mut tjs.runtime.host_mut().state;
        let stock: Vec<usize> = self.characters.iter().enumerate().filter(|(_, c)| !c.man && c.is_stock()).map(|(i, _)| i).collect();
        if !stock.iter().any(|i| state.maids.get(i).is_some_and(|m| m.leader)) {
            let pick = (0..=4).find_map(|s| self.active_maids.get(s).copied().flatten()).or_else(|| stock.first().copied());
            if let Some(i) = pick {
                state.maids.entry(i).or_default().leader = true;
            }
        }
        let p = &mut state.player;
        p.days = (p.days + 1).max(0);
        p.phase_days = (p.phase_days + 1).max(0);
        if p.flag("__ランキング解放") == 1 {
            let n = p.flag("__ランキング日数");
            p.generic_flag.insert("__ランキング日数".into(), n + 1);
        }
        // SATD-cm3d2-game-logic: ScheduleAPI.DayStartManage.
        Ok(())
    }

    /// `GameMain.OnEndDay`: fatigue and special condition reset, 疲労 when
    /// both HP and mind are spent, HP/mind restored; the ranking opens at
    /// five stock maids.
    fn on_end_day(&mut self) -> Result<()> {
        let mut tjs = self.adv.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let state = &mut tjs.runtime.host_mut().state;
        let stock: Vec<usize> = self.characters.iter().enumerate().filter(|(_, c)| !c.man && c.is_stock()).map(|(i, _)| i).collect();
        let data = state.data()?;
        let fatigue = crate::params::parse_enum(crate::params::FEATURE, "疲労").unwrap_or(19);
        for i in &stock {
            let m = state.maids.entry(*i).or_default();
            let mut p = crate::maid_param::MaidParam { st: m, data: &data };
            p.set_feature(fatigue, false);
            p.st.condition_special = 0;
            if p.st.a_cur_hp() <= 0 && p.st.a_cur_mind() <= 0 {
                p.set_feature(fatigue, true);
            }
            let (hp, mind) = (p.st.a_hp(), p.st.a_mind());
            p.set_cur_hp(hp);
            p.set_cur_mind(mind);
        }
        crate::tjs::update_maid_ranking(state);
        if stock.len() >= 5 {
            state.player.generic_flag.insert("__ランキング解放".into(), 1);
        }
        Ok(())
    }
}
