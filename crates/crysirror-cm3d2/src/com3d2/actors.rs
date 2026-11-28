// SPDX-License-Identifier: MPL-2.0
//! Retail CharacterMgr stock NPCs, named presets and active slots.
use super::*;
use crate::{
    chara::{Maid, View},
    kag::AdvUi,
    tjs::retail,
};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Actors {
    characters: Vec<Maid>,
    stock_indices: BTreeMap<usize,usize>,
    identities: BTreeMap<usize, usize>,
    pub slots: BTreeMap<(bool, usize), usize>,
    pub motions: BTreeMap<i32, Motion>,
    view: View,
    tracking: super::tracking::Tracking,
    despawn: Vec<u32>,
    position: [f32; 3],
    rotation: [f32; 3],
    group_offset: [f32; 3],
    group_rotation: [f32;3],
    last: Option<u64>,
    guest_man: Option<GuestMan>,
    native_men: Vec<retail::native_save::NativeMan>,
}

struct GuestMan {
    resume: Option<(usize, Maid)>,
    player_name: String,
}

pub(super) struct Motion {
    kag: AdvKag,
    maid: Option<usize>,
    man: Option<usize>,
    positions: bool,
}
impl Motion {
    fn update(&mut self, files: &GameFiles, op: &Opener, now: u64) -> Result<Vec<AdvUi>> {
        self.kag
            .update(now, false, false, files, op.storage.as_ref())?;
        let mut effects = std::mem::take(&mut self.kag.ui);
        effects.retain(|e| self.positions || !matches!(e, AdvUi::MaidOffset { multi: true, .. }));
        for e in &mut effects {
            match e {
                AdvUi::Motion { man, slot, .. }
                | AdvUi::RetailPropSet { man, slot, .. }
                | AdvUi::RetailPropReset { man, slot, .. }
                | AdvUi::ItemSet { man, slot, .. }
                | AdvUi::ItemReset { man, slot, .. }
                | AdvUi::ItemMask { man, slot, .. }
                | AdvUi::ItemMaskMode { man, slot, .. }
                | AdvUi::ItemChange { man, slot, .. }
                | AdvUi::AllProcProp { man, slot }
                | AdvUi::AutoTwist { man, slot, .. }
                | AdvUi::MotionStop { man, slot, .. }
                | AdvUi::BustMove { man, slot, .. }
                | AdvUi::PhysicsHit { man, slot, .. }
                | AdvUi::FaceBlend { man, slot, .. }
                | AdvUi::Eye { man, slot, .. }
                | AdvUi::HairPhysics { man, slot, .. }
                | AdvUi::MotionLayerStop { man, slot, .. }
                | AdvUi::Face { man, slot, .. }
                | AdvUi::Offset { man, slot, .. }
                    if *slot == 0 =>
                {
                    *slot = if *man { self.man } else { self.maid }.unwrap_or(0);
                }
                AdvUi::Ik { src, .. } if src.1 == 0 => {
                    src.1 = if src.0 { self.man } else { self.maid }.unwrap_or(0);
                }
                AdvUi::ManOffset { man, target, .. } => {
                    if *man == 0 { *man = self.man.unwrap_or(0); }
                    if target.1 == 0 { target.1 = if target.0 { self.man } else { self.maid }.unwrap_or(0); }
                }
                AdvUi::MaidOffset { list, .. } => {
                    for (s, _) in list {
                        if *s == 0 {
                            *s = self.maid.unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(effects)
    }
}

impl Actors {
    pub(super) fn native_men(&self) -> &[retail::native_save::NativeMan] {
        &self.native_men
    }

    pub(super) fn restore_native_roster(
        script: &AdvKag,
        men: Vec<retail::native_save::NativeMan>,
    ) -> Result<Self> {
        let mut actors = Self::default();
        actors.characters.resize_with(9, || Maid::new(false));
        for (stock, man) in men.iter().enumerate() {
            let index = 3 + stock;
            actors.characters[index] = man.model.clone();
            if let Some(slot) = man.active_slot() {
                actors.slots.insert((true, slot), index);
            }
        }
        let slots = script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?
            .runtime.host().state.active_maids.clone();
        for (slot, id) in slots.into_iter().enumerate() {
            if let Some(id) = id {
                actors.activate_stock(script, id, slot, Some(false))?;
            }
        }
        actors.native_men = men;
        Ok(actors)
    }

    pub fn photo_blink(&mut self, enabled: bool) -> Result<()> {
        self.actor(false, 0)
            .context("missing portrait subject")?
            .face
            .blink_disabled = !enabled;
        Ok(())
    }
    pub fn photo_gaze(
        &mut self,
        view: Option<&crysirror_core::graphics::WorldPhoto>,
    ) -> Result<()> {
        let index = *self
            .slots
            .get(&(false, 0))
            .context("missing portrait subject")?;
        self.tracking.photo_target =
            view.map(|v| (index, glam::Vec3::new(-v.eye[0], v.eye[1], v.eye[2])));
        self.tracking.gaze(index);
        Ok(())
    }
    fn group_frame(&self)->([f32;3],[f32;3]){
        let parent=crate::chara::unity_euler(self.rotation);
        let pos=glam::Vec3::from_array(self.position)+parent*glam::Vec3::from_array(self.group_offset);
        if self.group_rotation==[0.;3]{return (pos.to_array(),self.rotation);}
        let (y,x,z)=(parent*crate::chara::unity_euler(self.group_rotation)).to_euler(glam::EulerRot::YXZ);
        (pos.to_array(),[x,y,z].map(f32::to_degrees))
    }
    pub fn examination_head(&mut self,files:&GameFiles,op:&Opener,now:u64)->Result<[f32;3]> {
        let index=*self.slots.get(&(false,0)).context("missing examination subject")?;
        let actor=&self.characters[index];
        let (pos,rot)=self.group_frame();let group=crate::chara::unity_euler(rot);
        let translation=glam::Vec3::from_array(pos)+group*glam::Vec3::from_array(actor.position);
        let world=glam::Mat4::from_rotation_translation(group*crate::chara::unity_euler(actor.rotation),translation);
        Ok(self.tracking.bone_position(index,actor,"Bip01 Head",world,files,op,now)?.to_array())
    }
    pub fn photo_view(
        &mut self,
        kind: photo::Kind,
        orbit: Orbit,
        files: &GameFiles,
        op: &Opener,
        now: u64,
    ) -> Result<crysirror_core::graphics::WorldPhoto> {
        use glam::{Mat4, Quat, Vec3};
        let index = *self
            .slots
            .get(&(false, 0))
            .context("missing portrait subject")?;
        let m = &self.characters[index];
        let (eye, rotation, fov) = if let Some(offset) = kind.head_offset() {
            let (pos,rot)=self.group_frame();
            let group = crate::chara::unity_euler(rot);
            let transform = Mat4::from_rotation_translation(
                group * crate::chara::unity_euler(m.rotation),
                Vec3::from_array(pos) + group * Vec3::from_array(m.position),
            );
            let (eye, q) = self.tracking.photo_transform(
                index,
                m,
                transform,
                self.view.body_shapes(index),
                offset,
                files,
                op,
                now,
            )?;
            (eye, q, 30.)
        } else {
            let q = Quat::from_rotation_y(orbit.2[0].to_radians())
                * Quat::from_rotation_x(orbit.2[1].to_radians());
            (Vec3::from_array(orbit.0) - q * Vec3::Z * orbit.1, q, 35.)
        };
        let mirror = |v: Vec3| [-v.x, v.y, v.z];
        Ok(crysirror_core::graphics::WorldPhoto {
            size: kind.size(),
            eye: mirror(eye),
            target: mirror(eye + rotation * Vec3::Z),
            up: mirror(rotation * Vec3::Y),
            fov_y_degrees: fov,
            near: 0.01,
            far: 5.,
            entities: self.view.photo_parts(index),
        })
    }
    pub fn show_editor_stock(&mut self)->Result<()> {
        self.actor(false,0).context("missing live SceneEdit actor")?.visible=true;
        Ok(())
    }
    pub fn handles(op: &AdvUi) -> bool {
        matches!(
            op,
            AdvUi::RetailPropSet { .. } | AdvUi::RetailPropReset { .. } | AdvUi::ItemSet { .. } | AdvUi::ItemReset { .. } | AdvUi::ItemChange { .. } | AdvUi::ItemMask { .. } | AdvUi::ItemMaskMode { .. } | AdvUi::RetailDeactivate(_) | AdvUi::RetailSwap { .. } | AdvUi::RetailStock { .. } | AdvUi::AllProcProp { .. } | AdvUi::RetailNpc { .. }
                | AdvUi::Ik { target: None, .. }
                | AdvUi::CameraChase { .. }
                | AdvUi::CameraChaseStop
                | AdvUi::Eye { .. }
                | AdvUi::Chinko { .. }
                | AdvUi::AllOffset { .. }
                | AdvUi::AllPos { .. }
                | AdvUi::Offset { .. }
                | AdvUi::Motion { .. }
                | AdvUi::Face { .. }
                | AdvUi::AutoTwist { .. }
                | AdvUi::MotionStop { .. }
                | AdvUi::BustMove { .. }
                | AdvUi::PhysicsHit { .. }
                | AdvUi::FaceBlend { .. }
                | AdvUi::HairPhysics { .. }
                | AdvUi::MotionLayerStop { .. }
                | AdvUi::ManOffset { .. }
                | AdvUi::MaidOffset { .. }
                | AdvUi::CharaVisible { .. }
                | AdvUi::CharaVisibleAllOff { .. }
                | AdvUi::CharaDeactivate { .. }
        )
    }
    pub(super) fn activate_stock(&mut self, script:&AdvKag, id:usize, slot:usize, visible:Option<bool>)->Result<()> {
        ensure!(slot < 18, "invalid stock maid slot {slot}");
        let vm = script.tjs.lock().map_err(|_|anyhow::anyhow!("TJS lock"))?;
        let model = vm.runtime.host().state.retail.stock_models.get(&id).context("missing stock model")?;
        let index = if let Some(&index)=self.stock_indices.get(&id) {index} else {
            let index = self.characters.len().max(9);
            self.characters.resize_with(index, || Maid::new(false));
            self.characters.push((**model).clone());
            self.stock_indices.insert(id,index);
            index
        };
        self.slots.retain(|_,v|*v!=index);
        if let Some(old)=self.slots.insert((false,slot),index) {
            self.characters[old].visible=false;
            self.tracking.remove(old);
        }
        if let Some(visible)=visible {
            self.characters[index].visible=visible;
        }
        Ok(())
    }

    fn process_properties(&mut self, script:&AdvKag, files:&GameFiles, op:&Opener, man:bool, slot:usize)->Result<()> {
        let Some(&index)=self.slots.get(&(man,slot)) else {return Ok(())};
        let mut vm = script.tjs.lock().map_err(|_|anyhow::anyhow!("TJS lock"))?;
        let state=&mut vm.runtime.host_mut().state;
        let model=&mut self.characters[index];
        let id=self.stock_indices.iter().find_map(|(&id,&i)|(i==index).then_some(id))
            .or_else(||self.identities.get(&index).copied());
        if !man && let Some(id)=id && state.bodies.contains_key(&id) {
            retail::apply_live_body(state,id,model)?;
        }else{
            model.all_proc_prop(files,op.storage.as_ref())?;
        }
        Ok(())
    }
    fn hide_slot(&mut self, man: bool, slot: usize) {
        if let Some(i) = self.slots.remove(&(man, slot)) {
            self.tracking.remove(i);
            self.characters[i].tracking_rotations.clear();
            self.characters[i].visible = false;
        }
    }
    pub(super) fn voice_source(&self, man: bool, slot: usize) -> Option<[f32; 3]> {
        let actor = self.characters.get(*self.slots.get(&(man, slot))?)?;
        if !actor.visible {
            return None;
        }
        let (position, rotation) = self.group_frame();
        Some((glam::Vec3::from_array(position)
            + crate::chara::unity_euler(rotation) * glam::Vec3::from_array(actor.position)).to_array())
    }

    pub(super) fn actor(&mut self, man: bool, slot: usize) -> Option<&mut Maid> {
        self.characters.get_mut(*self.slots.get(&(man, slot))?)
    }

    /// KasizukiManager caches the player's appearance while using slot zero
    /// to present the currently selected guest.
    pub fn display_guest_man(
        &mut self,
        script: &AdvKag,
        files: &GameFiles,
        assets: &mut Assets,
        op: &mut Opener,
        player_model: Option<&Maid>,
        preset: Option<&str>,
        now: u64,
    ) -> Result<()> {
        if self.guest_man.is_none() {
            let vm = script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
            self.guest_man = Some(GuestMan {
                resume: self.slots.get(&(true, 0)).map(|&index| (index, self.characters[index].clone())),
                player_name: vm.runtime.host().state.player.player_name.clone(),
            });
        }
        let name = if let Some(preset) = preset {
            self.activate(script, files, assets, op, true, 0, preset, true)?;
            self.actor(true, 0).context("guest man slot")?.name_first.clone()
        } else {
            let guest = self.guest_man.as_ref().context("guest player appearance")?;
            let mut model = match player_model.or_else(|| guest.resume.as_ref().map(|(_, model)| model)) {
                Some(model) => model.clone(),
                None => {
                    let mut model = retail::retail_maid(true);
                    model.set_up_model(files, op.storage.as_ref(), "_I_man.menu")?;
                    model
                }
            };
            model.visible = true;
            model.name_last.clone_from(&guest.player_name);
            let name = guest.player_name.clone();
            let index = guest.resume.as_ref().map_or(3, |(index, _)| *index);
            if self.characters.len() <= index {
                self.characters.resize_with(index + 1, || Maid::new(false));
            }
            if let Some(old) = self.slots.insert((true, 0), index) {
                self.characters[old].visible = false;
                self.tracking.remove(old);
                self.despawn.extend(self.view.remove_actor(old));
            }
            self.characters[index] = model;
            name
        };
        self.actor(true, 0).context("guest man slot")?.chinko_visible = false;
        self.apply(
            AdvUi::Motion {
                man: true,
                slot: 0,
                file: "man_porse01.anm".into(),
                looping: true,
                blend_s: 0.,
                queue: false,
            },
            script,
            files,
            assets,
            op,
            now,
        )?;
        let mut vm = script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
        vm.runtime.host_mut().state.player.player_name = name;
        Ok(())
    }

    pub fn end_guest_man(&mut self, script: &AdvKag) -> Result<()> {
        let Some(guest) = self.guest_man.take() else {
            return Ok(());
        };
        self.hide_slot(true, 0);
        if let Some((index, model)) = guest.resume {
            self.characters[index] = model;
            self.slots.insert((true, 0), index);
        }
        let mut vm = script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
        vm.runtime.host_mut().state.player.player_name = guest.player_name;
        Ok(())
    }
    fn activate(
        &mut self,
        script: &AdvKag,
        files: &GameFiles,
        assets: &mut Assets,
        op: &mut Opener,
        man: bool,
        slot: usize,
        name: &str,
        load: bool,
    ) -> Result<()> {
        ensure!(
            slot < if man { 6 } else { 18 },
            "retail character slot {slot}"
        );
        let mut vm = script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let state = &mut vm.runtime.host_mut().state;
        let mut key = name.to_owned();
        if state.retail.compatibility {
            if matches!(name, "叔父メイド" | "メイド秘書" | "エステメイド") {
                key = format!("旧{name}");
            }
        } else {
            let sub = state.retail.table("maid_status_submaid_list.nei")?;
            if let Some(row) = (1..sub.rows).find(|&i| sub.string(1, i) == name) {
                ensure!(!man, "sub-maid in a man slot");
                key = sub.string(6, row);
                if state.player.flag("__lockUserDraftMaid") != 0 {
                    let other = state
                        .retail
                        .table("maid_status_submaid_kiss_side_list.nei")?;
                    if let Some(r) = (1..other.rows).find(|&i| other.int(0, i) == sub.int(0, row)) {
                        key = other.string(6, r);
                    }
                }
            }
        }
        let table = state.retail.table("character_preset_basedata.nei")?;
        let row = (1..table.rows)
            .find(|&r| table.string(0, r) == key)
            .with_context(|| format!("unknown NPC preset {key}"))?;
        let index = if man {
            3 + slot
        } else {
            (0..3)
                .find(|i| !self.slots.values().any(|v| v == i))
                .context("three retail NPC maids are active")?
        };
        let mut model = retail::retail_maid(man);
        model.npc = true;
        model.set_up_model(
            files,
            op.storage.as_ref(),
            if man {
                "_I_man.menu"
            } else {
                "_I_default.menu"
            },
        )?;
        model.name_last = table.string(1, row);
        model.name_first = table.string(2, row);
        let id = *self.identities.entry(index).or_insert_with(|| {
            let id = state.next_character;
            state.next_character += 1;
            id
        });
        if man {
            let file = table.string(4, row);
            ensure!(!file.is_empty(), "missing male NPC menu");
            model.set_up_model(
                files,
                op.storage.as_ref(),
                &format!("{}.menu", file.trim_end_matches(".menu")),
            )?;
        } else {
            let name = table.string(3, row);
            let stem = name.trim_end_matches(".preset");
            let file = format!("{stem}.preset");
            let bytes = if files.exists(&file) {
                files.read(op.storage.as_ref(), &file)?
            } else {
                let key = assets
                    .unity
                    .resource_of(
                        op,
                        &format!("Preset/{stem}"),
                        crysirror_unity::class_id::TEXT_ASSET,
                    )?
                    .with_context(|| format!("missing retail NPC preset {stem}"))?;
                let (_, data) = assets.unity.bytes(op, &key)?;
                crysirror_unity::classes::text_asset(&data)?.1
            };
            state.bodies.remove(&id);
            retail::apply_preset_bytes(state, id, &bytes)?;
            let body = &state.bodies[&id];
            for name in retail::RETAIL_MPN {
                if let Some(file) = body.menu_files.get(*name) {
                    model.set_prop_file(name, file, false)?;
                } else if let Some(&(kind, value)) = body.props.get(*name)
                    && kind != 3
                {
                    model.set_value(name, value)?;
                }
            }
            state.retail.apply_model_colors(id, &mut model);
            model.all_proc_prop(files, op.storage.as_ref())?;
            let body = state.bodies.get_mut(&id).context("NPC preset body")?;
            body.loaded_slots = model
                .slots
                .iter()
                .filter(|(_, s)| s.model.is_some())
                .map(|(name, _)| (*name).to_owned())
                .collect();
            body.params = model.slots.values().map(|s| s.params.clone()).collect();
            let guid = state.guid();
            let status = state.maids.entry(id).or_default();
            if status.guid_id.is_empty() {
                status.guid_id = guid;
            }
            status.first_name = model.name_first.clone();
            status.last_name = model.name_last.clone();
            state
                .active_maids
                .resize(state.active_maids.len().max(slot + 1), None);
            state.active_maids[slot] = Some(id);
        }
        while self.characters.len() <= index {
            self.characters.push(Maid::new(false));
        }
        if let Some(prev) = self.slots.insert((man, slot), index) {
            self.characters[prev].visible = false;
        }
        self.tracking.remove(index);
        self.despawn.extend(self.view.remove_actor(index));
        model.visible = load;
        self.characters[index] = model;
        Ok(())
    }

    pub fn clear_motion_stacks(&mut self) {
        for motion in self.motions.values_mut() { motion.kag.clear_call_stack(); }
    }
    pub fn start_motion(
        &mut self,
        script: &AdvKag,
        slot: i32,
        file: &str,
        label: &str,
        maid: Option<usize>,
        man: Option<usize>,
        positions: bool,
        files: &GameFiles,
        op: &Opener,
        now: u64,
    ) -> Result<Vec<AdvUi>> {
        let mut kag = AdvKag::motion(script.tjs.clone());
        kag.retail_tags = true;
        kag.enabled = true;
        kag.load(files, op.storage.as_ref(), file, label)?;
        kag.exec();
        let mut motion = Motion {
            kag,
            maid,
            man,
            positions,
        };
        let ops = motion.update(files, op, now)?;
        self.motions.insert(slot, motion);
        Ok(ops)
    }
    pub fn tick_motions(&mut self, files: &GameFiles, op: &Opener, now: u64) -> Result<Vec<AdvUi>> {
        let mut effects = Vec::new();
        for motion in self.motions.values_mut() {
            effects.extend(motion.update(files, op, now)?);
        }
        Ok(effects)
    }
    pub fn apply(
        &mut self,
        effect: AdvUi,
        script: &AdvKag,
        files: &GameFiles,
        assets: &mut Assets,
        op: &mut Opener,
        now: u64,
    ) -> Result<()> {
        match effect {
            AdvUi::RetailDeactivate(slot) => self.hide_slot(false,slot),
            AdvUi::RetailSwap {man,slot,other} => {
                let limit=if man {6} else {18};
                ensure!(slot<limit && other<limit,"invalid active slot swap");
                if let (Some(&a),Some(&b))=(self.slots.get(&(man,slot)),self.slots.get(&(man,other))) {
                    self.slots.insert((man,slot),b);
                    self.slots.insert((man,other),a);
                    if !man {
                        let mut vm=script.tjs.lock().map_err(|_|anyhow::anyhow!("TJS lock"))?;
                        let slots=&mut vm.runtime.host_mut().state.active_maids;
                        slots.swap(slot,other);
                    }
                }
            }
            AdvUi::RetailStock {index,slot,visible} => self.activate_stock(script,index,slot,visible)?,
            AdvUi::RetailPropSet {man, slot, category, value, temporary} => {
                if let Some(&index) = self.slots.get(&(man,slot)) {
                    let model = &mut self.characters[index];
                    model.set_numeric_prop(&category, value, temporary)?;
                    if !temporary && !man {
                        let id = self.stock_indices.iter().find_map(|(&id,&actor)| (index==actor).then_some(id))
                            .or_else(|| self.identities.get(&index).copied());
                        if let Some(id)=id {
                            let mut tjs=script.tjs.lock().map_err(|_|anyhow::anyhow!("TJS lock"))?;
                            let state=&mut tjs.runtime.host_mut().state;
                            if let Some(body)=state.bodies.get_mut(&id) {
                                let name = retail::RETAIL_MPN.iter().find(|n| n.eq_ignore_ascii_case(&category)).context("unknown numeric property")?;
                                if let Some((_,v))=body.props.get_mut(*name) {*v=model.value(name);}
                            }
                            if let Some(stock)=state.retail.stock_models.get_mut(&id) {
                                std::sync::Arc::make_mut(stock).set_value(&category,value)?;
                            }
                        }
                    }
                }
            }
            AdvUi::RetailPropReset {man,slot,category} => {
                if let Some(model)=self.actor(man,slot) {model.reset_numeric_prop(&category)?;}
            }
            AdvUi::ItemSet { man, slot, category, item, temp } => {
                if let Some(&index) = self.slots.get(&(man, slot)) {
                    let model = &mut self.characters[index];
                    model.set_prop_file(&category, &item, temp)?;
                    if !man && !temp {
                        let id = self.stock_indices.iter().find_map(|(&id,&actor)| (actor==index).then_some(id))
                            .or_else(|| self.identities.get(&index).copied());
                        if let Some(id) = id {
                            let mut tjs = script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
                            let state = &mut tjs.runtime.host_mut().state;
                            if let Some(body) = state.bodies.get_mut(&id) {
                                body.menu_files.insert(category.clone(), item.clone());
                            }
                            if let Some(stock) = state.retail.stock_models.get_mut(&id) {
                                std::sync::Arc::make_mut(stock).set_prop_file(&category, &item, false)?;
                            }
                        }
                    }
                }
            }
            AdvUi::ItemReset {man,slot,props} => {
                if let Some(model)=self.actor(man,slot) {
                    for name in props { model.reset_prop(&name)?; }
                }
            }
            AdvUi::ItemChange {man,slot,category,name} => {
                if let Some(model)=self.actor(man,slot) { model.item_change_temp(&category,&name)?; }
            }
            AdvUi::ItemMask {man,slot,body_slot,visible} => {
                if let Some(model)=self.actor(man,slot) {model.set_mask(&body_slot,visible)?;}
            }
            AdvUi::ItemMaskMode {man,slot,mode} => {
                if let Some(model)=self.actor(man,slot) {model.set_mask_mode(&mode)?;}
            }
            AdvUi::AllProcProp {man,slot} => self.process_properties(script,files,op,man,slot)?,

            AdvUi::RetailNpc {
                man,
                slot,
                name,
                load,
            } => self.activate(script, files, assets, op, man, slot, &name, load)?,
            AdvUi::CameraChase {
                man,
                slot,
                bone,
                offset,
                once,
                seconds,
            } => self.tracking.follow(
                self.slots.get(&(man, slot)).copied(),
                bone,
                offset,
                once,
                seconds,
                now,
            ),
            AdvUi::CameraChaseStop => self.tracking.stop(),
            AdvUi::Eye {
                man,
                slot,
                look,
                blend_s,
            } => {
                if let Some(&i) = self.slots.get(&(man, slot)) {
                    self.characters[i].eye = (look, blend_s);
                    self.tracking.gaze(i);
                }
            }
            AdvUi::CharaVisible { man, slot, visible } => {
                if let Some(m) = self.actor(man, slot) {
                    m.visible = visible;
                }
            }
            AdvUi::CharaVisibleAllOff { man } => {
                for m in self.characters.iter_mut().filter(|m| m.man == man) {
                    m.visible = false;
                }
            }
            AdvUi::CharaDeactivate { man, slot } => {
                self.hide_slot(man,slot);
                if !man {
                    let mut vm = script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
                    if let Some(i) = vm.runtime.host_mut().state.active_maids.get_mut(slot) {
                        *i = None;
                    }
                }
            }
            AdvUi::AllOffset {position,rotation}=>{
                if let Some(p)=position{self.group_offset=p;}
                if let Some(r)=rotation{self.group_rotation=r;}
            }
            AdvUi::AllPos { position, rotation } => {
                if let Some(p) = position {
                    self.position = p;
                }
                if let Some(r) = rotation {
                    self.rotation = r;
                }
            }
            AdvUi::Offset {
                man,
                slot,
                position,
                rotation,
            } => {
                if let Some(m) = self.actor(man, slot) {
                    if let Some(p) = position {
                        m.position = p;
                    }
                    if let Some(r) = rotation {
                        m.rotation = r;
                    }
                }
            }
            AdvUi::Ik {
                src,
                bone,
                target: None,
                ..
            } => {
                if let Some(m) = self.actor(src.0, src.1) {
                    m.ik.remove(&bone);
                }
            }
            AdvUi::MotionStop { man, slot } => {
                if let Some(m) = self.actor(man, slot) {
                    m.stop_anime();
                }
            }
            AdvUi::BustMove {
                man,
                slot,
                left,
                right,
            } => {
                if let Some(m) = self.actor(man, slot) {
                    if let Some(left) = left {
                        m.bust_move[0] = left;
                    }
                    if let Some(right) = right {
                        m.bust_move[1] = right;
                    }
                }
            }
            AdvUi::PhysicsHit { man, slot, height } => {
                if let Some(m) = self.actor(man, slot) {
                    m.hit_height = height;
                }
            }
            AdvUi::Chinko { slot, on } => {
                if let Some(m) = self.actor(true, slot) {
                    m.chinko_visible = on;
                }
            }
            AdvUi::FaceBlend { man, slot, name } => {
                if let Some(m) = self.actor(man, slot) {
                    m.face.blend(&name);
                }
            }
            AdvUi::Motion {
                man,
                slot,
                file,
                looping,
                blend_s,
                queue,
            } => {
                if let Some(m) = self.actor(man, slot) {
                    let bytes = files.read(op.storage.as_ref(), &file)?;
                    let clip = crysirror_asset::interchange::cm3d2::Animation::parse(&bytes)?;
                    let duration = clip
                        .tracks
                        .iter()
                        .flat_map(|t| t.curves.iter().flatten())
                        .flat_map(|c| c.keys.iter().map(|k| k[0]))
                        .fold(0., f32::max);
                    m.cross_fade(
                        &file,
                        looping,
                        queue,
                        if script.motion_blend { blend_s } else { 0. },
                        now,
                        duration,
                    );
                }
            }
            AdvUi::Face { man, slot, name } => {
                if let Some(m) = self.actor(man, slot) {
                    m.face.anime(&name, &m.blend_sets);
                }
            }
            AdvUi::AutoTwist { man, slot, twists } => {
                if let Some(m) = self.actor(man, slot) {
                    m.auto_twist.extend(twists);
                }
            }
            AdvUi::HairPhysics { man, slot, .. } => {
                if let Some(m) = self.actor(man, slot) {
                    m.hair_mode = "normal".into();
                }
            }
            AdvUi::MotionLayerStop { man, slot, layers } => {
                if let Some(m) = self.actor(man, slot) {
                    for l in layers {
                        ensure!(l >= 2, "motion layer");
                        m.stop_anime_layer(l as u32);
                    }
                }
            }
            AdvUi::ManOffset { man, target, split, per, extra_offsets } => {
                if let Some(&index) = self.slots.get(&(true, man)) {
                    ensure!(!extra_offsets, "unsupported retail supplemental bone offsets");
                    let target = *self.slots.get(&target).context("missing character offset reference")?;
                    self.characters[index].position = crate::adapter::maid_offset(&self.characters[target], split, &per).0;
                }
            }
            AdvUi::MaidOffset { list, split, multi } => {
                let mut max_height = 0f32;
                for (s, p) in list {
                    if let Some(m) = self.actor(false, s) {
                        let (offset, all) = crate::adapter::maid_offset(m, split, &p);
                        m.position = offset;
                        max_height = max_height.max(all[1]);
                    }
                }
                if multi {
                    self.group_offset = [0., max_height, 0.];
                }
            }
            other => bail!("unsupported retail character effect {other:?}"),
        }
        Ok(())
    }
    pub fn set_player_opacity(&mut self, opacity: f32) {
        for man in self.characters.iter_mut().filter(|m| m.man && m.man_alpha != opacity) {
            man.man_alpha = opacity;
            for slot in man.slots.values_mut() {
                slot.dirty = true;
            }
        }
    }

    pub fn sync(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        files: &GameFiles,
        op: &Opener,
        now: u64,
        camera: &mut Orbit,
        camera_offset: f32,
    ) -> Result<()> {
        if self.characters.is_empty() {
            return Ok(());
        }
        if self.view.shape_rules.is_none() && self.characters.iter().any(|m| m.visible && !m.man) {
            let path = crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, &format!("{}/Managed/Assembly-CSharp.dll",op.dir))?;
            let assembly = crysirror_dotnet::Assembly::parse(op.storage.read(&path)?)?;
            self.view.shape_rules = Some(crate::bone_shape::Rules::load(&assembly)?);
        }
        let dt = now.saturating_sub(self.last.replace(now).unwrap_or(now)) as f32 / 1000.;
        let (group_pos,group_rot)=self.group_frame();
        if self.tracking.update(
            &mut self.characters,
            &self.slots,
            glam::Vec3::from_array(group_pos),
            crate::chara::unity_euler(group_rot),
            camera,
            files,
            op,
            now,
            dt,
        )? {
            let mut command = crate::bg::orbit_camera_offset(camera.0, camera.2, camera.1, 35., camera_offset);
            command["op"] = json!("set_camera");
            host.submit_engine(client, 1, &serde_json::to_vec(&json!({"world":[command]}))?)?;
        }
        for m in self.characters.iter_mut().filter(|m| m.visible) {
            if !m.face.blink_disabled || !m.face.name2.is_empty() {
                m.morphs = m.face.update(dt, now as f32 / 1000., m.performance_audio, &m.blend_sets);
            }
        }
        let mut cmds = self
            .despawn
            .drain(..)
            .map(|id| json!({"op":"despawn","id":id}))
            .collect::<Vec<_>>();
        cmds.extend(self.view.sync(
            &mut self.characters,
            group_pos,
            group_rot,
            files,
            op.storage.as_ref(),
            now,
            &mut |id, b| {
                Ok(host
                    .upload_model(client, 0x45000000 + id, b)?
                    .relative()
                    .to_owned())
            },
            &Default::default(),
        )?);
        if !cmds.is_empty() {
            host.submit_engine(client, 1, &serde_json::to_vec(&json!({"world":cmds}))?)?;
        }
        self.release(host, client)
    }
    fn release(&mut self, host: &mut dyn HostServices, client: ClientId) -> Result<()> {
        for path in self.view.released.drain(..) {
            host.release_model(
                client,
                &crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, &path)?,
            )?;
        }
        Ok(())
    }
    /// DeactivateMaidAll retains slot zero. DeactivateManAll hides all men in place.
    pub fn reset_for_day(&mut self, state: &mut crate::tjs::TjsState) {
        self.slots.retain(|&(man, slot), _| man || slot == 0);
        for slot in state.active_maids.iter_mut().skip(1) { *slot = None; }
        self.reset_positions();
        self.tracking = Default::default();
        for actor in &mut self.characters {
            actor.visible = false;
            actor.tracking_rotations.clear();
        }
        self.motions.clear();
    }
    /// CharacterMgr.ResetCharaPosAll preserves assignments and ongoing motions.
    pub(super) fn reset_positions(&mut self) {
        self.position = [0.; 3];
        self.rotation = [0.; 3];
        self.group_offset = [0.; 3];
        self.group_rotation = [0.;3];
        for actor in &mut self.characters {
            actor.position = [0.; 3];
            actor.rotation = [0.; 3];
            actor.offset_position = [0.; 3];
            actor.hit_height = 0.;
        }
    }
    pub fn leave(&mut self, host: &mut dyn HostServices, client: ClientId) -> Result<()> {
        self.tracking.stop();
        self.despawn.extend(self.view.reset());
        let cmds: Vec<_> = self
            .despawn
            .drain(..)
            .map(|id| json!({"op":"despawn","id":id}))
            .collect();
        if !cmds.is_empty() {
            host.submit_engine(client, 1, &serde_json::to_vec(&json!({"world":cmds}))?)?;
        }
        self.release(host, client)
    }
}
