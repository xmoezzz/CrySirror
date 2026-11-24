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
    last: Option<u64>,
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
    pub fn show_editor_stock(&mut self)->Result<()> {
        self.actor(false,0).context("missing live SceneEdit actor")?.visible=true;
        Ok(())
    }
    pub fn handles(op: &AdvUi) -> bool {
        matches!(
            op,
            AdvUi::RetailDeactivate(_) | AdvUi::RetailSwap { .. } | AdvUi::RetailStock { .. } | AdvUi::AllProcProp { .. } | AdvUi::RetailNpc { .. }
                | AdvUi::Ik { target: None, .. }
                | AdvUi::CameraChase { .. }
                | AdvUi::CameraChaseStop
                | AdvUi::Eye { .. }
                | AdvUi::Chinko { .. }
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
                | AdvUi::MaidOffset { .. }
                | AdvUi::CharaVisible { .. }
                | AdvUi::CharaVisibleAllOff { .. }
                | AdvUi::CharaDeactivate { .. }
        )
    }
    fn activate_stock(&mut self, script:&AdvKag, id:usize, slot:usize, visible:Option<bool>)->Result<()> {
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
    fn actor(&mut self, man: bool, slot: usize) -> Option<&mut Maid> {
        self.characters.get_mut(*self.slots.get(&(man, slot))?)
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
    pub fn sync(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        files: &GameFiles,
        op: &Opener,
        now: u64,
        camera: &mut Orbit,
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
        let group_pos = std::array::from_fn(|i| self.position[i] + self.group_offset[i]);
        if self.tracking.update(
            &mut self.characters,
            &self.slots,
            glam::Vec3::from_array(group_pos),
            crate::chara::unity_euler(self.rotation),
            camera,
            files,
            op,
            now,
            dt,
        )? {
            let mut command = crate::bg::orbit_camera(camera.0, camera.2, camera.1, 35.);
            command["op"] = json!("set_camera");
            host.submit_engine(client, 1, &serde_json::to_vec(&json!({"world":[command]}))?)?;
        }
        for m in self.characters.iter_mut().filter(|m| m.visible) {
            m.morphs = m.face.update(dt, now as f32 / 1000., None, &m.blend_sets);
        }
        let mut cmds = self
            .despawn
            .drain(..)
            .map(|id| json!({"op":"despawn","id":id}))
            .collect::<Vec<_>>();
        cmds.extend(self.view.sync(
            &mut self.characters,
            std::array::from_fn(|i| self.position[i] + self.group_offset[i]),
            self.rotation,
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
        self.position = [0.; 3];
        self.rotation = [0.; 3];
        self.group_offset = [0.; 3];
        self.tracking = Default::default();
        for actor in &mut self.characters {
            actor.visible = false;
            actor.position = [0.; 3];
            actor.rotation = [0.; 3];
            actor.offset_position = [0.; 3];
            actor.hit_height = 0.;
            actor.tracking_rotations.clear();
        }
        self.motions.clear();
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
