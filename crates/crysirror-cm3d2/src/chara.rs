// SPDX-License-Identifier: MPL-2.0
//! `Maid`/`TBody`/`TBodySkin` (Assembly-CSharp): props, item menus and body
//! slots, projected as world entities that share the body skeleton.
//!
//! - `Maid.SetProp(category, menu)` records an item; `AllProcPropSeq` runs
//!   each changed item menu (MPN order) through `Menu.ProcScriptBin` until
//!   no item menu sets another (`アイテム` inside an item menu).
//! - `TBody.AddItem` loads a `.model` into a slot; parts follow the body's
//!   same-named bones (`TBodySkin.CopyTrans`) — here `AnimationState.follow`
//!   — and their roots ride the slot's attach bone (`TBodySkin.Load`
//!   parenting) — `AnimationState.attach`.
//! - Material, texture and colour edits (`ChangeMaterial`, `ChangeTex`,
//!   `ChangeCol`) are applied when the slot's model is converted.
//! - `node消去`/`node表示` mark body bones whose names contain the node name,
//!   and their descendants, per slot (`TBodySkin.SetVisibleFlag`, replayed in
//!   command order; `additem` resets the slot with `_ALL_`). Bones hidden by
//!   any visible slot drop the body triangles weighted to them
//!   (`TBody.FixVisibleFlag` → `TMorph.FixVisibleFlag`).
//! - `maskitem` hides another slot while the masking category has an item
//!   (`TBody.AddMask`/`FixMaskFlag`).
//! - `blendset` defines face expressions (`TMorph.NewBlendSet`/
//!   `SetValueBlendSet`) applied as morph weights of the head slot.
use crate::{files::GameFiles, menu::MenuFile};
use anyhow::{Context, Result};
use crysirror_asset::interchange::cm3d2::{LayerBlend, Material, MaterialEdits, Mesh, TextureLayer};
use crysirror_core::service::ClientStorage;
use std::collections::BTreeMap;

/// `MPN` in enum order (`m_aryMaidProp` processing order).
pub const MPN: &[&str] = &[
    "null_mpn", "MuneL", "MuneTare", "RegFat", "ArmL", "Hara", "RegMeet", "KubiScl", "UdeScl", "EyeScl", "EyePosX", "EyePosY",
    "HeadX", "HeadY", "DouPer", "sintyou", "koshi", "kata", "west", "MuneUpDown", "MuneYori", "body", "moza", "head", "hairf",
    "hairr", "hairt", "hairs", "haircolor", "skin", "acctatoo", "underhair", "hokuro", "mayu", "lip", "eye", "eye_hi", "chikubi",
    "chikubicolor", "wear", "skirt", "mizugi", "bra", "panz", "stkg", "shoes", "headset", "glove", "acchead", "hairaho", "accha",
    "acchana", "acckamisub", "acckami", "accmimi", "accnip", "acckubi", "acckubiwa", "accheso", "accude", "accashi", "accsenaka",
    "accshippo", "accanl", "accvag", "megane", "accxxx", "handitem", "acchat", "onepiece", "set_maidwear", "set_mywear",
    "set_underwear", "folder_eye", "folder_mayu", "folder_underhair", "folder_skin", "kousoku_upper", "kousoku_lower",
    "seieki_naka", "seieki_hara", "seieki_face", "seieki_mune", "seieki_hip", "seieki_ude", "seieki_ashi",
];

/// `TBody.m_strDefSlotName`: slot, attach bone, visibility group.
pub const SLOTS: &[(&str, &str)] = &[
    ("body", "_ROOT_"), ("head", "Bip01 Head"), ("eye", "Bip01 Head"), ("hairF", "Bip01 Head"), ("hairR", "Bip01 Head"),
    ("hairS", "Bip01 Head"), ("hairT", "Bip01 Head"), ("wear", "_ROOT_"), ("skirt", "_ROOT_"), ("onepiece", "_ROOT_"),
    ("mizugi", "_ROOT_"), ("panz", "_ROOT_"), ("bra", "_ROOT_"), ("stkg", "_ROOT_"), ("shoes", "_ROOT_"),
    ("headset", "Bip01 Head"), ("glove", "_ROOT_"), ("accHead", "Bip01 Head"), ("hairAho", "Bip01 Head"), ("accHana", "_ROOT_"),
    ("accHa", "Bip01 Head"), ("accKami_1_", "Bip01 Head"), ("accMiMiR", "Bip01 Head"), ("accKamiSubR", "Bip01 Head"),
    ("accNipR", "_ROOT_"), ("HandItemR", "_IK_handR"), ("accKubi", "Bip01 Spine1a"), ("accKubiwa", "Bip01 Neck"),
    ("accHeso", "Bip01 Head"), ("accUde", "_ROOT_"), ("accAshi", "_ROOT_"), ("accSenaka", "_ROOT_"), ("accShippo", "Bip01 Spine"),
    ("accAnl", "_ROOT_"), ("accVag", "_ROOT_"), ("kubiwa", "_ROOT_"), ("megane", "Bip01 Head"), ("accXXX", "_ROOT_"),
    ("chinko", "Bip01 Pelvis"), ("chikubi", "_ROOT_"), ("accHat", "Bip01 Head"), ("kousoku_upper", "_ROOT_"),
    ("kousoku_lower", "_ROOT_"), ("seieki_naka", "_ROOT_"), ("seieki_hara", "_ROOT_"), ("seieki_face", "_ROOT_"),
    ("seieki_mune", "_ROOT_"), ("seieki_hip", "_ROOT_"), ("seieki_ude", "_ROOT_"), ("seieki_ashi", "_ROOT_"),
    ("accNipL", "_ROOT_"), ("accMiMiL", "Bip01 Head"), ("accKamiSubL", "Bip01 Head"), ("accKami_2_", "Bip01 Head"),
    ("accKami_3_", "Bip01 Head"), ("HandItemL", "_IK_handL"), ("underhair", "_ROOT_"), ("moza", "_ROOT_"),
];

fn slot_name(name: &str) -> Option<&'static str> {
    SLOTS.iter().map(|(s, _)| *s).find(|s| s.eq_ignore_ascii_case(name))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Slot {
    pub model: Option<String>,
    pub edits: MaterialEdits,
    /// `node消去`/`node表示` of this slot's menus in order
    /// (`m_dicDelNodeBody`, keyed by body bone name when replayed).
    pub nodes: Vec<(String, bool)>,
    pub params: BTreeMap<String, String>,
    /// Material → parts colour of its `_MultiColTex` (`ChangeTex` with a
    /// `PARTS_COLOR`).
    pub multicolor: BTreeMap<usize, usize>,
    /// `additem … アタッチ <slot> <point>` or `ボーンにアタッチ <bone>`.
    pub attach: Option<(String, String)>,
    /// `アタッチポイントの設定` of this slot's model: name → (position, Euler degrees).
    pub attach_points: BTreeMap<String, ([f32; 3], [f32; 3])>,
    /// `TBodySkin.listMaskSlot` (`maskitem` of this slot's menus).
    pub mask_slots: Vec<&'static str>,
    /// Converted model needs rebuilding.
    pub dirty: bool,
}

/// A Unity legacy `AnimationState` of the body (`TBody.m_Bones.animation`):
/// weight fades linearly from `w0` at `since` toward `target` at `rate`/s.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimState {
    /// `GetAnimeTag`: the lower-case file name.
    pub tag: String,
    pub file: String,
    /// `_l_N_` (2..=8) in the tag, else 0.
    pub layer: u32,
    pub w0: f32,
    pub since: u64,
    pub target: f32,
    pub rate: f32,
    /// Clip time 0 (ms).
    pub start: u64,
    pub looping: bool,
    /// Seconds.
    pub duration: f32,
    /// `CrossFadeQueued` fade, until the layer's others complete.
    pub queued: Option<f32>,
}

impl AnimState {
    pub fn weight(&self, now: u64) -> f32 {
        let dt = now.saturating_sub(self.since) as f32 / 1000.;
        if self.w0 < self.target {
            (self.w0 + self.rate * dt).min(self.target)
        } else {
            (self.w0 - self.rate * dt).max(self.target)
        }
    }
    /// A `WrapMode.Once` clip has played to its end.
    pub fn finished(&self, now: u64) -> bool {
        !self.looping && now.saturating_sub(self.start) as f32 / 1000. >= self.duration
    }
    fn retarget(&mut self, now: u64, target: f32, fade_s: f32) {
        self.w0 = self.weight(now);
        self.since = now;
        self.target = target;
        self.rate = if fade_s > 0. { 1. / fade_s } else { 1000. };
    }
}

/// `TBody.LoadAnime` layer: `_l_N_` for N in 2..=8.
fn anime_layer(tag: &str) -> u32 {
    (2..=8).find(|i| tag.contains(&format!("_l_{i}_"))).unwrap_or(0)
}

/// `MaidProp`: one entry of `Maid.m_aryMaidProp`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prop {
    /// 0 none, 1 vertex morph, 2 bone morph, 3 item menu.
    pub kind: u8,
    pub min: i32,
    pub max: i32,
    pub default: i32,
    pub value: i32,
    pub file: String,
    pub temp_file: String,
    /// `boDut` / `boTempDut`.
    pub dut: bool,
    pub temp_dut: bool,
}

/// `Maid.Initialize` numeric props: `(min, max, default, kind)` of MPN 1..=20.
const NUMERIC_PROPS: [(i32, i32, i32, u8); 20] = [
    (0, 130, 10, 1), (0, 130, 10, 1), (0, 100, 40, 1), (0, 100, 20, 1), (0, 100, 20, 1), (0, 100, 40, 1),
    (20, 80, 50, 2), (0, 100, 50, 2), (0, 100, 50, 2), (0, 100, 50, 2), (0, 100, 50, 2), (0, 100, 50, 2),
    (0, 100, 50, 2), (20, 80, 50, 2), (20, 80, 50, 2), (0, 100, 50, 2), (0, 100, 50, 2), (0, 100, 50, 2),
    (0, 100, 10, 2), (0, 100, 40, 2),
];

/// MPN index by name (case-insensitive, as `m_hashMaidPropLower`).
pub fn mpn(name: &str) -> Option<usize> {
    MPN.iter().position(|m| m.eq_ignore_ascii_case(name))
}

#[derive(Clone, Debug, Default)]
pub struct Maid {
    pub(crate) prop_names: &'static [&'static str],
    pub man: bool,
    /// `m_aryMaidProp`, indexed by MPN.
    pub props: Vec<Prop>,
    pub slots: BTreeMap<&'static str, Slot>,
    /// `TBody.m_hFoceHide` overrides (default visible) and `m_eMaskMode`.
    pub force_hide: BTreeMap<&'static str, bool>,
    pub mask_mode: String,
    /// Face expressions: set name → morph weights.
    pub blend_sets: BTreeMap<String, BTreeMap<String, f32>>,
    pub face: crate::face::FaceState,
    /// Head morph weights of this frame.
    pub morphs: BTreeMap<String, f32>,
    pub(crate) tracking_rotations: BTreeMap<String,[f32;4]>,
    pub body_morphs: BTreeMap<String, f32>,
    /// `Maid.SetAutoTwist` switches by `AutoTwist` name.
    pub auto_twist: BTreeMap<&'static str, bool>,
    /// `SetModeHairR`, `MuneYureL/R`, `SetBoneHitHeightY`.
    pub hair_mode: String,
    pub bust_move: [f32; 2],
    pub hit_height: f32,
    /// IK targets by hand bone (`IKTargetToBone` / `IKTargetToAttachPoint`).
    pub ik: BTreeMap<String, ((bool, usize), crate::kag::IkTarget, [f32; 3])>,
    /// `Maid.AddPrefab` instances: name → (prefab, bone, local position, Euler).
    pub prefabs: BTreeMap<String, (String, String, [f32; 3], [f32; 3])>,
    /// `Maid.m_vOffsetPostion` (`SetPosOffsetFromUdeScl*`).
    pub offset_position: [f32; 3],
    /// `Maid.EyeTo*` state: target and fade seconds.
    pub eye: (crate::kag::EyeLook, f32),
    /// `Maid.ManColor`.
    pub man_color: [f32; 4],
    /// `CMSystem.ManAlpha` / 100 (`_FloatValue2` of `CM3D2/Man`).
    pub man_alpha: f32,
    pub visible: bool,
    pub position: [f32; 3],
    /// Euler degrees, Unity order.
    pub rotation: [f32; 3],
    /// `TBody` animation states.
    pub anims: Vec<AnimState>,
    pub name_first: String,
    pub name_last: String,
    pub npc: bool,
    /// `CharacterMgr.BanishmentMaid`: no longer a stock maid (kept so
    /// character indices stay valid).
    pub banished: bool,
    /// `TBody.SetChinkoVisible`.
    pub chinko_visible: bool,
    /// `MaidParts` colours by `PARTS_COLOR`.
    pub parts_color: [crate::parts_color::PartsColor; 7],
    pub(crate) outline_colors: Option<[crate::parts_color::PartsColor; 2]>,
    /// `Menu.m_dicResourceRef`: menu file (lower case) → key → menu
    /// (`リソース参照`, `半脱ぎ`).
    pub resource_refs: BTreeMap<String, BTreeMap<String, String>>,
    /// `MaidParam.SetPartsValue` (`commenttype`).
    pub parts_comments: BTreeMap<String, String>,
    /// `TMorph.NewParamSet` (`paramset`) of the face.
    pub face_params: Vec<Vec<String>>,
    /// `Debug.LogWarning` of menu processing, drained into the session log.
    pub warnings: Vec<String>,
}

/// Upper bound of `AllProcPropSeq` passes (item menus setting item menus).
const MAX_PASSES: usize = 16;

impl Maid {
    pub fn new(man: bool) -> Self {
        // Maid.Initialize AddProp: numeric MPN 1..=20, items (with the
        // `CM3.dicDelItem` menu as default file) after; every item dirty.
        let props = MPN
            .iter()
            .enumerate()
            .map(|(i, name)| match i {
                0 => Prop { max: i32::MAX, ..Default::default() },
                1..=20 => {
                    let (min, max, default, kind) = NUMERIC_PROPS[i - 1];
                    Prop { kind, min, max, default, value: default, ..Default::default() }
                }
                _ => Prop { kind: 3, file: crate::preset::del_item(name).unwrap_or("").to_owned(), dut: true, ..Default::default() },
            })
            .collect();
        // Maid.m_colMan default.
        Self {
            man,
            prop_names: MPN,
            props,
            man_color: [0., 0.09, 1., 1.],
            man_alpha: 0.5,
            chinko_visible: true,
            parts_color: crate::parts_color::defaults(),
            face: crate::face::FaceState::new(),
            mask_mode: "None".into(),
            hair_mode: "normal".into(),
            bust_move: [1., 1.],
            eye: (crate::kag::EyeLook::Camera("目と顔を向ける".into()), 0.5),
            ..Default::default()
        }
    }

    fn prop_index(&self, name: &str) -> Option<usize> {
        self.prop_names.iter().position(|p| p.eq_ignore_ascii_case(name))
    }

    /// A stock character (not an NPC, not dismissed).
    pub fn is_stock(&self) -> bool {
        !self.npc && !self.banished
    }

    /// Numeric prop value by MPN name (`GetProp(..).value`).
    pub fn value(&self, name: &str) -> i32 {
        self.prop_index(name).and_then(|i| self.props.get(i)).map_or(0, |p| p.value)
    }

    /// Item menu of a prop (`strFileName`).
    pub fn file(&self, name: &str) -> Option<&str> {
        self.prop_index(name).and_then(|i| self.props.get(i)).map(|p| p.file.as_str()).filter(|f| !f.is_empty())
    }

    /// `Maid.SetProp(MPN, int)`: clamped to the prop's range.
    pub fn set_value(&mut self, name: &str, value: i32) -> Result<()> {
        let i = self.prop_index(name).with_context(|| format!("指定されたプロパティ名がありません。{name}"))?;
        let p = &mut self.props[i];
        p.value = value.clamp(p.min, p.max);
        p.dut = true;
        Ok(())
    }

    /// `Maid.SetProp(MaidProp, filename, rid, temp)`.
    pub fn set_prop_file(&mut self, name: &str, file: &str, temp: bool) -> Result<()> {
        let i = self.prop_index(name).with_context(|| format!("指定されたプロパティ名がありません。{name}"))?;
        let p = &mut self.props[i];
        if temp {
            if file != p.temp_file {
                p.temp_dut = true;
                p.temp_file = file.to_owned();
            }
        } else {
            if p.file != file || matches!(self.prop_names[i], "set_maidwear" | "set_mywear" | "set_underwear" | "set_body") {
                p.dut = true;
            }
            p.file = file.to_owned();
            p.temp_file.clear();
        }
        Ok(())
    }

    /// `Maid.DelProp`: the prop's delete menu.
    pub fn del_prop(&mut self, name: &str, temp: bool) -> Result<()> {
        match crate::preset::del_item(&name.to_ascii_lowercase()) {
            Some(del) => self.set_prop_file(name, del, temp),
            None => Ok(()),
        }
    }

    /// `Maid.ResetProp`: drop the temporary item, reprocess the original.
    pub fn reset_prop(&mut self, name: &str) -> Result<()> {
        let i = self.prop_index(name).with_context(|| format!("指定されたプロパティ名がありません。{name}"))?;
        let p = &mut self.props[i];
        p.dut = true;
        p.temp_file.clear();
        p.temp_dut = false;
        Ok(())
    }

    /// `Maid.ItemChangeTemp`: the `リソース参照` alternative of the prop's
    /// current menu as a temporary item.
    pub fn item_change_temp(&mut self, name: &str, key: &str) -> Result<()> {
        let i = self.prop_index(name).with_context(|| format!("指定されたプロパティ名がありません。{name}"))?;
        let file = self.props[i].file.to_lowercase();
        let file = file.rsplit('/').next().unwrap_or(&file).to_owned();
        if let Some(alt) = self.resource_refs.get(&file).and_then(|r| r.get(key)).cloned() {
            self.set_prop_file(name, &alt, true)?;
        }
        Ok(())
    }

    /// `TBody.CrossFade` (or `CrossFadeQueued` with `queue`): the clip
    /// fades in over `fade_s` and the other states of its layer fade out.
    pub fn cross_fade(&mut self, file: &str, looping: bool, queue: bool, fade_s: f32, now: u64, duration: f32) {
        self.advance_anims(now);
        let tag = file.to_lowercase();
        let layer = anime_layer(&tag);
        let mut state = AnimState { tag: tag.clone(), file: file.to_owned(), layer, w0: 0., since: now, target: 0., rate: 0., start: now, looping, duration, queued: None };
        if queue {
            state.queued = Some(fade_s);
            self.anims.push(state);
            return;
        }
        self.anims.retain(|a| a.tag != tag || a.queued.is_some());
        for a in self.anims.iter_mut().filter(|a| a.layer == layer && a.queued.is_none()) {
            a.retarget(now, 0., fade_s);
        }
        state.retarget(now, 1., fade_s);
        self.anims.push(state);
    }

    /// `TBody.StopAnime("")`: every state.
    pub fn stop_anime(&mut self) {
        self.anims.clear();
    }

    /// `TBody.StopAnimeLayer`.
    pub fn stop_anime_layer(&mut self, layer: u32) {
        self.anims.retain(|a| a.layer != layer);
    }

    /// Fades and queues at `now`: faded-out states go; a queued clip starts
    /// once every other state of its layer has completed.
    pub fn advance_anims(&mut self, now: u64) {
        self.anims.retain(|a| a.queued.is_some() || a.target > 0. || a.weight(now) > 0.);
        while let Some(i) = self.anims.iter().position(|q| {
            q.queued.is_some() && self.anims.iter().all(|a| a.layer != q.layer || a.queued.is_some() || a.finished(now))
        }) {
            let fade = self.anims[i].queued.take().unwrap_or(0.);
            let layer = self.anims[i].layer;
            for a in self.anims.iter_mut().filter(|a| a.layer == layer && a.queued.is_none()) {
                a.retarget(now, 0., fade);
            }
            let a = &mut self.anims[i];
            a.start = now;
            a.w0 = 0.;
            a.since = now;
            a.retarget(now, 1., fade);
            self.anims.retain(|a| a.queued.is_some() || a.target > 0. || a.weight(now) > 0.);
        }
    }

    /// Any item prop waiting to be processed (`AllProcPropSeqStart` busy).
    pub fn busy(&self) -> bool {
        self.props.iter().any(|p| p.kind == 3 && (p.dut || p.temp_dut))
    }

    /// `MaidParts.SetPartsColor`: slots tinted by that colour rebuild.
    pub fn set_parts_color(&mut self, id: usize, color: crate::parts_color::PartsColor) {
        let color_slot = if id < self.parts_color.len() {
            self.parts_color.get_mut(id)
        } else {
            self.outline_colors.as_mut().and_then(|colors| colors.get_mut(id - self.parts_color.len()))
        };
        let Some(value) = color_slot else { return };
        if *value == color { return; }
        *value = color;
        for s in self.slots.values_mut() {
            if s.multicolor.values().any(|v| *v == id) {
                s.dirty = true;
            }
        }
    }

    fn slot(&mut self, name: &str) -> Option<&mut Slot> {
        let s = slot_name(name)?;
        Some(self.slots.entry(s).or_default())
    }

    /// `Maid.SetProp(category, filename)` (non-temporary).
    pub fn set_prop(&mut self, category: &str, menu: &str) {
        let _ = self.set_prop_file(category, menu, false);
    }

    /// `Menu.SetMaidItemTemp`: the menu's own category selects the prop.
    fn set_item(&mut self, files: &GameFiles, storage: &dyn ClientStorage, menu: &str, temp: bool) -> Result<()> {
        let m = MenuFile::parse(&files.read(storage, menu)?).with_context(|| format!("menu {menu}"))?;
        self.set_prop_file(&m.category, menu, temp)
    }

    /// `Maid.SetUpModel`: a preset menu, then every item it selects.
    pub fn set_up_model(&mut self, files: &GameFiles, storage: &dyn ClientStorage, preset: &str) -> Result<()> {
        self.proc_script(files, storage, preset, false)?;
        self.all_proc_prop(files, storage).map(|_| ())
    }

    /// `Maid.AllProcProp`: item props in MPN order until none is dirty
    /// (head/hair/skin changes re-dirty their colour props), then the
    /// numeric props. Returns whether a model changed.
    pub fn all_proc_prop(&mut self, files: &GameFiles, storage: &dyn ClientStorage) -> Result<bool> {
        let mut changed = false;
        for _ in 0..MAX_PASSES {
            if !self.busy() {
                for p in &mut self.props {
                    if p.kind == 1 || p.kind == 2 {
                        p.dut = false;
                    }
                }
                return Ok(changed);
            }
            for i in 0..self.props.len() {
                if self.props[i].kind == 3 && self.proc_item(files, storage, i)? {
                    changed = true;
                    let prop_name = self.prop_names[i];
                    let mut redirty = |names: &[&str]| {
                        for n in names {
                            if let Some(k) = self.prop_index(n) {
                                self.props[k].dut = true;
                            }
                        }
                    };
                    match prop_name {
                        "hairf" | "hairr" | "hairt" | "hairs" | "hairaho" => redirty(&["haircolor"]),
                        "head" => redirty(&["skin", "eye", "mayu", "eye_hi"]),
                        "skin" => redirty(&["acctatoo", "hokuro", "lip"]),
                        _ => {}
                    }
                }
            }
        }
        anyhow::bail!("item menus keep selecting each other")
    }

    /// `Maid.ProcItem` for an item prop: the temporary menu, then the
    /// original; outfit and body sets clear themselves after running.
    fn proc_item(&mut self, files: &GameFiles, storage: &dyn ClientStorage, i: usize) -> Result<bool> {
        let mut result = false;
        if self.props[i].temp_dut {
            let file = self.props[i].temp_file.clone();
            if !file.is_empty() {
                self.proc_script_logged(files, storage, &file, true)?;
            }
            result = true;
            self.props[i].temp_dut = false;
            if matches!(self.prop_names[i], "set_maidwear" | "set_mywear" | "set_underwear" | "set_body") {
                self.props[i].temp_file.clear();
            }
        }
        if self.props[i].dut {
            let file = self.props[i].file.clone();
            if !file.is_empty() {
                self.proc_script_logged(files, storage, &file, false)?;
            }
            result = true;
            self.props[i].dut = false;
            if matches!(self.prop_names[i], "set_maidwear" | "set_mywear" | "set_underwear" | "set_body") {
                self.props[i].file.clear();
            }
        }
        Ok(result)
    }

    /// `Menu.ProcScript`: a missing menu only warns (`LogWarning`).
    fn proc_script_logged(&mut self, files: &GameFiles, storage: &dyn ClientStorage, file: &str, temp: bool) -> Result<()> {
        if !files.exists(file) {
            self.warnings.push(format!("アイテムメニューファイルが見つかりません。{file}"));
            return Ok(());
        }
        self.proc_script(files, storage, file, temp)
    }

    /// `Menu.ProcScriptBin`; `temp` is its `f_bTemp`.
    pub fn proc_script(&mut self, files: &GameFiles, storage: &dyn ClientStorage, file: &str, temp: bool) -> Result<()> {
        let menu = MenuFile::parse(&files.read(storage, file)?).with_context(|| format!("menu {file}"))?;
        let mut category = String::new();
        let mut slot_for_attach = String::new();
        let num = |s: &str| s.trim().parse::<f32>().unwrap_or(0.);
        let mut params = Vec::new();
        for c in &menu.commands {
            let Some(head) = c.first() else { continue };
            let arg = |i: usize| c.get(i).map(String::as_str).unwrap_or("");
            match head.as_str() {
                "アイテム" => self.set_item(files, storage, arg(1), temp)?,
                "アイテム条件" => {
                    let slot = slot_name(arg(1)).and_then(|s| self.slots.get(s)).cloned().unwrap_or_default();
                    let (ok, item) = match arg(2) {
                        "に何か" => (if arg(3) == "有る" { slot.model.is_some() } else { slot.model.is_none() }, arg(5)),
                        "が" => (slot.model.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(arg(3))), arg(5)),
                        "のアイテムパラメータの" => (slot.params.get(&arg(3).to_ascii_lowercase()).is_some_and(|v| *v == arg(5).to_ascii_lowercase()), arg(7)),
                        _ => (false, ""),
                    };
                    if ok && !item.is_empty() {
                        self.set_item(files, storage, item, false)?;
                    }
                }
                "アイテムパラメータ" if c.len() == 4 => params.push((arg(1).to_owned(), arg(2).to_ascii_lowercase(), arg(3).to_ascii_lowercase())),
                "setslotitem" | "prop" => self.set_value(arg(1), arg(2).trim().parse::<i64>().unwrap_or(0) as i32)?,
                "category" => category = arg(1).to_owned(),
                "additem" => {
                    let slot = if c.len() > 2 { arg(2).to_owned() } else { category.clone() };
                    slot_for_attach = slot.clone();
                    let attach = match (c.len(), arg(3)) {
                        (6, "アタッチ") => Some((arg(4).to_owned(), arg(5).to_owned())),
                        (5, "ボーンにアタッチ") => Some(("ボーンにアタッチ".to_owned(), arg(4).to_owned())),
                        _ if slot.eq_ignore_ascii_case("handitemr") => Some(("ボーンにアタッチ".to_owned(), "_IK_handR".to_owned())),
                        _ if slot.eq_ignore_ascii_case("handiteml") => Some(("ボーンにアタッチ".to_owned(), "_IK_handL".to_owned())),
                        _ => None,
                    };
                    if let Some(s) = self.slot(&slot) {
                        // TBodySkin.Load → DeleteObj, then SetVisibleNodeSlot(_ALL_, true).
                        *s = Slot { model: Some(arg(1).to_owned()), dirty: true, attach, ..Default::default() };
                    }
                }
                "delitem" => {
                    let slot = if c.len() > 1 { arg(1).to_owned() } else { category.clone() };
                    if let Some(s) = self.slot(&slot) {
                        // TBodySkin.DeleteObj keeps `m_dicDelNodeBody`.
                        let nodes = std::mem::take(&mut s.nodes);
                        let had = s.model.is_some();
                        *s = Slot { nodes, dirty: had, ..Default::default() };
                    }
                }
                // TBody.AddMask(category, maskslot).
                "maskitem" if c.len() > 1 => {
                    if let (Some(owner), Some(masked)) = (slot_name(&category), slot_name(arg(1))) {
                        self.slots.entry(owner).or_default().mask_slots.push(masked);
                    }
                }
                "node消去" | "node表示" => {
                    let show = head == "node表示";
                    if let Some(s) = self.slot(&category) {
                        s.nodes.push((arg(1).to_owned(), show));
                    }
                }
                "color" => {
                    let rgba = [num(arg(4)) / 255., num(arg(5)) / 255., num(arg(6)) / 255., num(arg(7)) / 255.];
                    if let Some(s) = self.slot(arg(1)) {
                        s.edits.colors.push((arg(2).parse().unwrap_or(0), arg(3).to_owned(), rgba));
                        s.dirty = true;
                    }
                }
                "mancolor" => self.man_color = [num(arg(1)) / 255., num(arg(2)) / 255., num(arg(3)) / 255., 1.],
                // TBody.ChangeTex: maids only, a loaded slot only; `*` is the
                // model stem; a PARTS_COLOR binds `_MultiColTex`; layers of
                // the property are removed (MulTexRemove).
                "tex" | "テクスチャ変更" if !self.man => {
                    let color = if c.len() == 6 {
                        Some(crate::parts_color::id(arg(5)).or_else(|| {
                            if self.prop_names == MPN { return None; }
                            match arg(5) { "HAIR_OUTLINE" => Some(7), "SKIN_OUTLINE" => Some(8), _ => None }
                        }).with_context(|| format!("無限色IDがありません。{}", arg(5)))?)
                    } else {
                        None
                    };
                    if let Some(s) = self.slot(arg(1)).filter(|s| s.model.is_some()) {
                        let mat: usize = arg(2).parse().unwrap_or(0);
                        let stem = s.model.as_deref().map(|m| m.rsplit_once('.').map_or(m, |(a, _)| a)).unwrap_or("").to_owned();
                        let file = arg(4).replace('*', &stem);
                        s.edits.layers.retain(|l| !(l.material == mat && l.property == arg(3)));
                        s.edits.textures.push((mat, arg(3).to_owned(), file));
                        s.edits.multicolor.insert(mat, color.is_some());
                        match color {
                            Some(id) => s.multicolor.insert(mat, id),
                            None => s.multicolor.remove(&mat),
                        };
                        s.dirty = true;
                    }
                }
                "テクスチャ合成" if !self.man => {
                    anyhow::ensure!(c.len() == 7, "テクスチャ合成 の引数が不正です。{}", c.len());
                    let blend = match arg(6) {
                        "Alpha" => LayerBlend::Alpha,
                        "Multiply" => LayerBlend::Multiply,
                        other => anyhow::bail!("テクスチャ合成: blend {other}"),
                    };
                    if let Some(s) = self.slot(arg(1)) {
                        let (material, layer): (usize, i32) = (arg(2).parse().unwrap_or(0), arg(4).parse().unwrap_or(0));
                        s.edits.layers.retain(|l| !(l.material == material && l.property == arg(3) && l.layer == layer));
                        s.edits.layers.push(TextureLayer { material, property: arg(3).to_owned(), layer, file: arg(5).to_owned(), blend, placement: None });
                        s.dirty = true;
                    }
                }
                "アタッチポイントの設定" if c.len() >= 5 => {
                    let rot = if c.len() == 8 { [num(arg(5)), num(arg(6)), num(arg(7))] } else { [0.; 3] };
                    if let Some(s) = self.slot(&slot_for_attach) {
                        s.attach_points.insert(arg(1).to_owned(), ([num(arg(2)), num(arg(3)), num(arg(4))], rot));
                    }
                }
                "リソース参照" | "半脱ぎ" => {
                    let (k, v) = if head == "半脱ぎ" { ("半脱ぎ", arg(1)) } else { (arg(1), arg(2)) };
                    let key = file.rsplit('/').next().unwrap_or(file).to_lowercase();
                    self.resource_refs.entry(key).or_default().insert(k.to_owned(), v.to_owned());
                }
                "commenttype" => {
                    self.parts_comments.insert(arg(1).to_owned(), arg(2).to_owned());
                }
                "paramset" => self.face_params.push(c.clone()),
                "マテリアル変更" => {
                    let mate = Material::parse_file(&files.read(storage, arg(3))?).with_context(|| format!("material {}", arg(3)))?;
                    if let Some(s) = self.slot(arg(1)) {
                        s.edits.replace.insert(arg(2).parse().unwrap_or(0), mate);
                        s.dirty = true;
                    }
                }
                "blendset" => {
                    let mut set = BTreeMap::new();
                    for pair in c[2..].chunks(2) {
                        if let [k, v] = pair {
                            // TMorph.SetValueBlendSet stores val * 0.01.
                            set.insert(k.to_ascii_lowercase(), num(v) * 0.01);
                        }
                    }
                    self.blend_sets.insert(arg(1).to_owned(), set);
                }
                "end" => break,
                // Editor/catalogue metadata and attach points (handled with
                // their consumers; SATD-cm3d2-game-logic for the rest).
                _ => {}
            }
        }
        for (slot, key, value) in params {
            if let Some(s) = self.slot(&slot) {
                s.params.insert(key, value);
            }
        }
        Ok(())
    }

    /// `Maid.ManColor` + `ManColorUpdate`: the man shader colour of every slot.
    pub fn set_man_color(&mut self, color: [f32; 4]) {
        if self.man_color != color {
            self.man_color = color;
            for s in self.slots.values_mut() {
                s.dirty = true;
            }
        }
    }

    /// `TBody.FixMaskFlag` (maids): in slot order, a slot hidden by
    /// `m_hFoceHide` or by an earlier visible slot's mask list masks nothing;
    /// a visible slot hides its mask list. Slots without a model are not
    /// drawn. Men have no masks.
    pub fn visible_slots(&self) -> Vec<&'static str> {
        let mut visible: BTreeMap<&'static str, bool> = SLOTS.iter().map(|(n, _)| (*n, true)).collect();
        if !self.man {
            for (name, _) in SLOTS {
                if !self.force_hide.get(name).copied().unwrap_or(true) {
                    visible.insert(name, false);
                }
                if !visible[name] {
                    continue;
                }
                if let Some(s) = self.slots.get(name) {
                    for m in &s.mask_slots {
                        visible.insert(m, false);
                    }
                }
            }
        }
        self.slots.iter().filter(|(n, s)| s.model.is_some() && visible.get(*n).copied().unwrap_or(true)).map(|(n, _)| *n).collect()
    }

    /// `TBody.MulTexSet(.., Alpha, add, x, y, r, s)` (maids only): a placed
    /// layer drawn by the next `MulTexProc` (model rebuild).
    pub fn tex_mul_add(&mut self, slot: &str, material: usize, property: &str, layer: i32, file: &str, at: (i32, i32, f32, f32)) {
        if self.man {
            return;
        }
        if let Some(s) = self.slot(slot) {
            s.edits.layers.push(TextureLayer { material, property: property.to_owned(), layer, file: file.to_owned(), blend: LayerBlend::Alpha, placement: Some(at) });
            s.dirty = true;
        }
    }

    /// `TBody.MulTexRemove(slot, mat, prop, n)` for layers 10..=20.
    pub fn tex_mul_del(&mut self, slot: &str, material: usize, property: &str) {
        if let Some(s) = self.slot(slot) {
            let before = s.edits.layers.len();
            s.edits.layers.retain(|l| !(l.material == material && l.property == property && (10..=20).contains(&l.layer)));
            if s.edits.layers.len() != before {
                s.dirty = true;
            }
        }
    }

    /// `TBody.SetMask(slot, visible)` (slots after `eye` only).
    pub fn set_mask(&mut self, slot: &str, visible: bool) -> Result<()> {
        let i = SLOTS.iter().position(|(n, _)| n.eq_ignore_ascii_case(slot)).with_context(|| format!("@itemmask TBody.SlotID enum convert error.\n{slot}"))?;
        if i > 2 {
            self.force_hide.insert(SLOTS[i].0, visible);
        }
        Ok(())
    }

    /// `TBody.SetMaskMode`.
    pub fn set_mask_mode(&mut self, mode: &str) -> Result<()> {
        anyhow::ensure!(matches!(mode, "None" | "Swim" | "Underwear" | "Nude"), "@itemmaskmode TBody.MaskMode enum convert error.\n{mode}");
        self.mask_mode = mode.to_owned();
        for (n, _) in SLOTS {
            self.force_hide.insert(n, true);
        }
        let hide = |m: &mut Self, names: &[&'static str]| {
            for n in names {
                m.force_hide.insert(n, false);
            }
        };
        if mode == "Nude" {
            hide(self, &["stkg"]);
        }
        if mode == "Nude" || mode == "Underwear" {
            hide(self, &["mizugi", "onepiece"]);
        }
        if matches!(mode, "Nude" | "Underwear" | "Swim") {
            hide(self, &["wear", "skirt", "shoes", "headset", "glove", "accKubi", "accKubiwa"]);
        }
        if mode == "Nude" || mode == "Swim" {
            hide(self, &["bra", "panz"]);
        }
        Ok(())
    }
}

/// `TBody.FixVisibleFlag` for the body: per skin bone of `body`, whether
/// no visible slot hides it. Each slot's commands are replayed over the body
/// bone hierarchy (`TBodySkin.SetVisibleFlag`: a node whose name contains
/// the fragment, or `_ALL_`, and all its descendants take the flag).
pub fn body_skin_visibility(body: &Mesh, slots: &[&Slot]) -> Vec<bool> {
    let mut visible = vec![true; body.skin_bones.len()];
    for slot in slots {
        let mut flags: Vec<Option<bool>> = vec![None; body.bones.len()];
        for (fragment, show) in &slot.nodes {
            for (i, flag) in flags.iter_mut().enumerate() {
                let mut at = Some(i);
                while let Some(j) = at {
                    if fragment == "_ALL_" || body.bones[j].name.contains(fragment.as_str()) {
                        *flag = Some(*show);
                        break;
                    }
                    at = body.bones[j].parent;
                }
            }
        }
        for (k, name) in body.skin_bones.iter().enumerate() {
            let hidden = body.bones.iter().position(|b| &b.name == name).and_then(|i| flags[i]) == Some(false);
            if hidden {
                visible[k] = false;
            }
        }
    }
    visible
}

struct DrawnPart {
    id: u32,
    asset: String,
    bones: Vec<crysirror_asset::interchange::cm3d2::Bone>,
    /// Native MPN and the matching blend shape on this particular mesh.
    numeric_morphs: Vec<(usize, String)>,
    bone_shapes: Vec<crate::bone_shape::Binding>,
}

/// World projection of the character manager's maids and men.
#[derive(Default)]
pub struct View {
    pub(crate) shape_rules: Option<crate::bone_shape::Rules>,
    parts: BTreeMap<(usize, &'static str), DrawnPart>,
    /// Last submitted animation state per entity.
    states: BTreeMap<u32, String>,
    visible: BTreeMap<usize, bool>,
    /// Node commands of the visible slots the body was built with.
    body_nodes: BTreeMap<usize, Vec<(&'static str, Vec<(String, bool)>)>>,
    next_model: u32,
    /// Uploaded model assets no longer drawn (despawned or rebuilt parts),
    /// for `HostServices::release_model`.
    pub released: Vec<String>,
}

impl View {
    /// Recycle one character while retaining the other actors on the stage.
    pub(crate) fn remove_actor(&mut self, index: usize) -> Vec<u32> {
        let mut children = Vec::new();
        let mut body = None;
        self.parts.retain(|&(actor, slot), part| {
            if actor != index { return true; }
            self.states.remove(&part.id);
            self.released.push(std::mem::take(&mut part.asset));
            if slot == "body" { body = Some(part.id); } else { children.push(part.id); }
            false
        });
        self.body_nodes.remove(&index);
        self.visible.remove(&index);
        children.extend(body);
        children
    }

    /// Forget every character entity (indices changed, e.g. by a load);
    /// returns the entities to despawn.
    pub fn reset(&mut self) -> Vec<u32> {
        // Parts are children of their body: children first.
        let (bodies, parts): (Vec<_>, Vec<_>) = self.parts.iter().partition(|(k, _)| k.1 == "body");
        let ids = parts.into_iter().chain(bodies).map(|(_, p)| p.id).collect();
        let assets: Vec<String> = self.parts.values().map(|p| p.asset.clone()).collect();
        self.released.extend(assets);
        self.parts.clear();
        self.states.clear();
        self.visible.clear();
        self.body_nodes.clear();
        ids
    }
}

/// Bytes of character models built per frame.
const UPLOAD_BUDGET: usize = 48 * 1024 * 1024;

/// Entity ids: characters own `0x1000 + 0x100 · index` upward.
fn entity(index: usize, slot: &str) -> u32 {
    let s = SLOTS.iter().position(|(n, _)| *n == slot).unwrap_or(0) as u32;
    0x1000 + 0x100 * index as u32 + s
}

/// Unity `Quaternion.Euler` (Z, then X, then Y).
pub fn unity_euler(d: [f32; 3]) -> glam::Quat {
    let r = d.map(f32::to_radians);
    glam::Quat::from_rotation_y(r[1]) * glam::Quat::from_rotation_x(r[0]) * glam::Quat::from_rotation_z(r[2])
}

/// The engine transform of an offset under the character root
/// (`CharacterMgr` all-position/rotation), mirrored to engine space.
pub fn world_transform(all_pos: [f32; 3], all_rot: [f32; 3], pos: [f32; 3], rot: [f32; 3]) -> serde_json::Value {
    let qa = unity_euler(all_rot);
    let q = qa * unity_euler(rot);
    let p = glam::Vec3::from_array(all_pos) + qa * glam::Vec3::from_array(pos);
    let m = glam::Quat::from_xyzw(q.x, -q.y, -q.z, q.w);
    let (x, y, z) = m.to_euler(glam::EulerRot::XYZ);
    serde_json::json!({"translation": [-p.x, p.y, p.z], "rotation_degrees": [x.to_degrees(), y.to_degrees(), z.to_degrees()], "scale": [1., 1., 1.]})
}

impl View {
    /// Build changed slot models (GLB via `upload`) and return world commands.
    #[allow(clippy::too_many_arguments)]
    pub fn sync(
        &mut self,
        characters: &mut [Maid],
        all_pos: [f32; 3],
        all_rot: [f32; 3],
        files: &GameFiles,
        storage: &dyn ClientStorage,
        now: u64,
        upload: &mut dyn FnMut(u32, &[u8]) -> Result<String>,
        resources: &std::collections::HashMap<String, Vec<u8>>,
    ) -> Result<Vec<serde_json::Value>> {
        use serde_json::json;
        let mut out = Vec::new();
        // Models are built for visible characters only, and at most
        // UPLOAD_BUDGET bytes of them per frame; the rest stay dirty.
        let mut uploaded = 0usize;
        for (index, maid) in characters.iter_mut().enumerate() {
            let body_id = entity(index, "body");
            if !maid.visible && !self.parts.keys().any(|(i, _)| *i == index) {
                continue;
            }
            let visible_slots = maid.visible_slots();
            let (is_man, man_color, parts_color, man_alpha) = (maid.man, maid.man_color, maid.parts_color, maid.man_alpha);
            let outlines = maid.outline_colors;
            // TBody.FixVisibleFlag reruns when a visible slot's nodes change.
            let node_sig: Vec<(&'static str, Vec<(String, bool)>)> = visible_slots
                .iter()
                .filter_map(|s| maid.slots.get(s).filter(|x| !x.nodes.is_empty()).map(|x| (*s, x.nodes.clone())))
                .collect();
            if self.body_nodes.get(&index) != Some(&node_sig) {
                if let Some(b) = maid.slots.get_mut("body") {
                    b.dirty = true;
                }
            }
            let shown: Vec<&Slot> = visible_slots.iter().filter_map(|s| maid.slots.get(s)).collect();
            let node_slots: Vec<Slot> = shown.into_iter().cloned().collect();
            for (&slot, state) in maid.slots.iter_mut() {
                let present = state.model.is_some() && visible_slots.contains(&slot);
                let key = (index, slot);
                if !state.dirty && (present == self.parts.contains_key(&key)) {
                    continue;
                }
                if present && uploaded >= UPLOAD_BUDGET {
                    continue;
                }
                state.dirty = false;
                // Despawning a body recursively despawns its attachments. Preserve
                // their live entities while replacing the body model in this batch.
                let mut children = Vec::new();
                if slot == "body" && present && self.parts.contains_key(&key) {
                    for ((owner, part), drawn) in &self.parts {
                        let id = &drawn.id;
                        if *owner == index && *part != "body" {
                            children.push(*id);
                            out.push(json!({"op":"set_parent","id":id,"parent":null}));
                        }
                    }
                    self.visible.remove(&index);
                }
                if let Some(DrawnPart { id, asset, .. }) = self.parts.remove(&key) {
                    out.push(json!({"op": "despawn", "id": id}));
                    self.released.push(asset);
                    self.states.remove(&id);
                }
                if !present {
                    continue;
                }
                let model = state.model.clone().unwrap();
                let bytes = files.read(storage, &model)?;
                let mut mesh = Mesh::parse(&bytes).with_context(|| format!("model {model}"))?;
                if slot == "body" {
                    let refs: Vec<&Slot> = node_slots.iter().collect();
                    let visible = body_skin_visibility(&mesh, &refs);
                    mesh.hide_skin_bones(&visible);
                    self.body_nodes.insert(index, node_sig.clone());
                }
                let bones = mesh.bones.clone();
                let mut numeric_morphs = Vec::new();
                for (mpn, (&name, prop)) in maid.prop_names.iter().zip(&maid.props).enumerate() {
                    if prop.kind != 1 || is_man && !name.eq_ignore_ascii_case("hara") { continue; }
                    let native = if is_man { "karadal" } else { name };
                    if let Some(target) = mesh.morphs.iter().find(|target| target.name.eq_ignore_ascii_case(native)) {
                        numeric_morphs.push((mpn, target.name.clone()));
                    }
                }
                // `res:<path>` textures are Unity `Resources/Texture/<path>`.
                let mut external = |name: &str| match name.strip_prefix("res:") {
                    Some(_) => resources.get(name).cloned().with_context(|| format!("Resources texture {name}")),
                    None => files.read(storage, name),
                };
                let mut edits = state.edits.clone();
                for (mat, id) in &state.multicolor {
                    let color = parts_color.get(*id).or_else(|| outlines.as_ref()?.get(id.checked_sub(parts_color.len())?)).context("missing material color in character profile")?;
                    edits.color_tables.insert(*mat, crate::parts_color::table(color));
                }
                if is_man {
                    // TBodySkin.ManColorUpdate on `CM3D2/Man` materials;
                    // CMSystem.ManAlpha is the man's opacity.
                    for (i, m) in mesh.materials.iter().enumerate() {
                        if m.shader == "CM3D2/Man" {
                            let mut c = man_color;
                            c[3] = man_alpha;
                            edits.colors.push((i, "_Color".into(), c));
                        }
                    }
                }
                let doc = crysirror_asset::interchange::cm3d2::convert_parsed(mesh, &mut external, &edits)
                    .with_context(|| format!("model {model}"))?;
                self.next_model += 1;
                let glb = doc.to_glb()?;
                uploaded += glb.len();
                let asset = upload(self.next_model, &glb)?;
                let id = entity(index, slot);
                let mut spec = json!({"id": id, "name": format!("chara{index}/{slot}"), "geometry": {"kind": "scene", "asset": asset}});
                if slot != "body" {
                    spec["parent"] = json!(body_id);
                }
                out.push(json!({"op": "spawn", "entity": spec}));
                let bone_shapes = self.shape_rules.as_ref().filter(|_| !is_man).map_or_else(Vec::new, |r| r.bind(&bones));
                self.parts.insert(key, DrawnPart { id, asset, bones, numeric_morphs, bone_shapes });
                for child in children {
                    out.push(json!({"op":"set_parent","id":child,"parent":id}));
                }
            }
            // Transform, visibility, motion, hidden nodes and face per frame.
            if !self.parts.contains_key(&(index, "body")) {
                continue;
            }
            if self.visible.get(&index) != Some(&maid.visible) {
                out.push(json!({"op": "set_visible", "id": body_id, "visible": maid.visible}));
                self.visible.insert(index, maid.visible);
            }
            let transform = world_transform(all_pos, all_rot, maid.position, maid.rotation);
            let morphs = maid.morphs.clone();
            for ((i, slot), part) in &self.parts {
                let (id, bones) = (&part.id, &part.bones);
                if *i != index {
                    continue;
                }
                let mut nodes = serde_json::Map::new();
                for (name, q) in &maid.tracking_rotations {
                    if (*slot == "body" && !name.starts_with("Eye_")) || (*slot == "head" && name.starts_with("Eye_")) {
                        nodes.insert(name.clone(), json!({"rotation":q}));
                    }
                }
                for binding in &part.bone_shapes { binding.apply(maid, &mut nodes); }
                // TBody.SetChinkoVisible: `chinkoCenter` scale 0 in every slot.
                if !maid.chinko_visible && bones.iter().any(|b| b.name == "chinkoCenter") {
                    nodes.insert("chinkoCenter".into(), json!({"scale": [0., 0., 0.]}));
                }
                let mut anim = serde_json::Map::new();
                if *slot == "body" {
                    maid.advance_anims(now);
                    let layers: Vec<serde_json::Value> = maid
                        .anims
                        .iter()
                        .filter(|a| a.queued.is_none())
                        .filter_map(|a| {
                            let path = files.path(&a.file)?;
                            Some(json!({
                                "clip": crysirror_asset::interchange::CM3D2_CLIP, "source": path,
                                "time": now.saturating_sub(a.start) as f32 / 1000., "speed": 1.0, "looping": a.looping,
                                "weight": a.weight(now), "target_weight": a.target, "fade": a.rate.min(1000.),
                                "blend_layer": a.layer,
                            }))
                        })
                        .collect();
                    if !layers.is_empty() {
                        anim.insert("layers".into(), json!(layers));
                    }
                } else {
                    anim.insert("follow".into(), json!(body_id));
                    // TBodySkin.Load: the part root is parented to the slot's
                    // bone (`Bip01` → `ManBip` for men) at its rest offset.
                    if let Some((_, bone)) = SLOTS.iter().find(|(n, _)| n == slot).filter(|(_, b)| *b != "_ROOT_") {
                        let bone = if maid.man { bone.replace("Bip01", "ManBip") } else { (*bone).to_owned() };
                        anim.insert("attach".into(), json!(bone));
                    }
                }
                let mut weights: BTreeMap<_, _> = part.numeric_morphs.iter()
                    .map(|(mpn, name)| (name.clone(), maid.props[*mpn].value as f32 / 100.))
                    .collect();
                if *slot == "body" { weights.extend(maid.body_morphs.clone()); }
                if *slot == "head" { weights.extend(morphs.clone()); }
                if !weights.is_empty() { anim.insert("morphs".into(), json!(weights)); }
                if !nodes.is_empty() {
                    anim.insert("nodes".into(), serde_json::Value::Object(nodes));
                }
                // The world advances clip time; resubmit only on change of
                // the clip, follow, nodes or face (time excluded).
                let mut signature = serde_json::Value::Object(anim.clone());
                if let Some(ls) = signature.get_mut("layers").and_then(|l| l.as_array_mut()) {
                    for l in ls {
                        l["time"] = json!(0);
                        l["weight"] = json!(0);
                    }
                }
                let sig = format!("{signature}{transform}");
                if self.states.get(id) != Some(&sig) {
                    out.push(json!({"op": "set_animation", "id": id, "animation": serde_json::Value::Object(anim)}));
                    if *slot == "body" {
                        out.push(json!({"op": "set_transform", "id": id, "transform": transform}));
                    }
                    self.states.insert(*id, sig);
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod actor_lifetime {
    use super::*;

    #[test]
    fn replacing_a_pooled_actor_releases_only_its_drawn_parts() {
        let mut view = View::default();
        for (actor, slot) in [(0, "body"), (0, "head"), (1, "body")] {
            let id = entity(actor, slot);
            view.parts.insert((actor, slot), DrawnPart { id, asset: format!("{id}.glb"), bones: vec![], numeric_morphs: vec![], bone_shapes: vec![] });
            view.states.insert(id, String::from("previous pose"));
        }
        assert_eq!(view.remove_actor(0), vec![entity(0,"head"), entity(0,"body")]);
        assert!(view.states.contains_key(&entity(1,"body")));
        assert_eq!(view.parts.len(), 1);
        assert_eq!(view.released.len(), 2);
        assert!(view.remove_actor(0).is_empty());
    }
}
