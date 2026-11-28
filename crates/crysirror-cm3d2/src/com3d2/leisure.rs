// SPDX-License-Identifier: MPL-2.0
//! Standalone SceneCall destinations from DaytimeMain and NightMain.
use super::*;
use crate::tjs::retail::activities;
use std::collections::BTreeSet;
mod blackjack;
mod casino;
mod casino_shop;
mod examination;
mod room;
mod studio;
mod slot_machine;
mod native_menus;
pub(super) use native_menus::{FreeSkillLayout, VipLayout, is_free_skill};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Destination {
    Photography,
    CreativeRoom,
    Examination,
    Casino,
    CasinoShop,
    Blackjack,
    SlotMachine,
    FreeMode,
    GuestClub,
    Yotogi,
    OldYotogi,
}
use Destination::*;
impl Destination {
    pub fn name(self) -> &'static str {
        match self {
            Photography => "ScenePhotoMode",
            CreativeRoom => "SceneCreativeRoom",
            Examination => "SceneCompetitiveShow",
            Casino => "SceneCasino",
            CasinoShop => "SceneCasinoShop",
            Blackjack => "SceneBlackJack",
            SlotMachine => "SceneCasinoSlot",
            FreeMode => "SceneFreeModeSelect",
            GuestClub => "SceneKasizukiMainMenu",
            Yotogi => "SceneYotogi",
            OldYotogi => "SceneYotogiOld",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [Photography, CreativeRoom, Examination, Casino, CasinoShop, Blackjack, SlotMachine, FreeMode, GuestClub, Yotogi, OldYotogi]
            .into_iter()
            .find(|d| d.name() == s)
    }
}

#[derive(Default)]
pub(super) struct Screen {
    model_generation: u32,
    base: Vec<Widget>,
    pressed: Option<usize>,
    pub(super) dialog: Option<quit_dialog::QuitDialog>,
    response: Option<Response>,
    studio: studio::Studio,
    casino: casino::CasinoMenu,
    casino_shop: casino_shop::Shop,
    blackjack: blackjack::Table,
    machine: slot_machine::Machine,
    native_menu: native_menus::Menu,
    room: room::Room,
    examination: examination::Show,
}
#[derive(Clone, Copy)]
enum Response {
    Exit,
    Dismiss,
    Venue,
    Simplify,
    EndShow,
    RoomFile(usize, bool),
    CasinoBuy(usize),
    BlackjackBet(bool),
    BlackjackExit,
    SlotExit,
    GuestTitle,
}
pub(super) use studio::photo_widget;

impl Game {
    fn leisure_prompt(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
        now: u64,
        message: &str,
        style: &str,
        response: Response,
    ) -> Result<()> {
        let dialog = self.assets.scene(
            op,
            host,
            c,
            if style == "yesno" {
                "YesNoDialog"
            } else {
                "SystemDialog"
            },
        )?;
        let d = quit_dialog::QuitDialog::show(&mut self.widgets, dialog, now, message)?;
        if style == "ok" {
            let first = d.first;
            let mut idx = 0;
            self.widgets.retain(|w| {
                let keep = idx < first || !w.path.split('/').any(|n| n == "Cancel");
                idx += 1;
                keep
            });
        }
        self.leisure.dialog = Some(d);
        self.leisure.response = Some(response);
        Ok(())
    }
    pub(super) fn open_leisure(
        &mut self,
        d: Destination,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<(String, (Vec<u8>, usize))> {
        self.leisure = Screen::default();
        self.adv_cover = None;
        self.world_cover = None;
        self.dialogue = None;
        self.camera_move = None;
        self.message_open = false;
        self.camera_control = d != Examination;
        {
            let mut vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let s = &mut vm.runtime.host_mut().state;
            if is_free_skill(d, &self.script.tag_backup) {
                self.actors.reset_positions();
            } else {
                self.actors.reset_for_day(s);
                self.actors.slots.retain(|&(man, _), _| man);
            }
            if !matches!(d, Casino | CasinoShop | Blackjack | SlotMachine | FreeMode | GuestClub | Yotogi | OldYotogi) {
                s.active_maids.fill(None);
            }
        }
        self.sound.stop(host, c, Kind::Voice, 0.)?;
        self.leisure.base = std::mem::take(&mut self.widgets);
        let name = match d {
            Photography => self.open_studio()?,
            CreativeRoom => return self.open_room(op, host, c),
            Examination => self.open_examination(op, host, c, now)?,
            Casino => self.open_casino(op, host, c, now)?,
            CasinoShop => self.open_casino_shop(op, host, c, now)?,
            Blackjack => self.open_blackjack(op, host, c, now)?,
            SlotMachine => self.load_slot_machine(op, host, c, now)?,
            FreeMode | GuestClub | Yotogi | OldYotogi => self.open_native_menu(d, op, host, c, now)?,
        };
        Ok((
            name.clone(),
            club::prefab(&mut self.assets.unity, op, &name)?,
        ))
    }
    pub(super) fn update_leisure(
        &mut self,
        d: Destination,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
        input: InputFrame,
        released: u64,
        now: u64,
    ) -> Result<()> {
        if d == GuestClub && self.update_guest_comment(host, c, input)? {
            return Ok(());
        }
        if matches!(d, Examination | Casino | CasinoShop | Blackjack) {
            let effects = self.actors.tick_motions(&self.files, op, now)?;
            self.edit_motion_effects(effects, op, now)?;
        }
        if let Some(dialog) = self.leisure.dialog.as_mut() {
            if let Some(choice) = dialog.update(&self.widgets, input, released, now) {
                self.widgets.truncate(dialog.first);
                self.leisure.dialog = None;
                let ok = choice == quit_dialog::Choice::Ok;
                match self
                    .leisure
                    .response
                    .take()
                    .context("missing dialog callback")?
                {
                    Response::Exit if ok => {
                        if d == CreativeRoom {
                            self.sound.stop(host, c, Kind::Bgm, 1.)?;
                        }
                        self.resume_script(op, host, c, now)?;
                    }
                    Response::EndShow if ok => self.resume_script(op, host, c, now)?,
                    Response::RoomFile(slot, save) if ok => {
                        self.apply_room_file(slot, save, op, host, c)?
                    }
                    Response::Venue => {
                        self.casino_venue(if ok { 0 } else { 1 }, op, host, c, now)?
                    }
                    Response::Simplify => self.start_judging(ok, op, host, c, now)?,
                    Response::CasinoBuy(item) if ok => self.buy_casino_item(item, op, host, c, now)?,
                    Response::BlackjackBet(warn) if ok => self.blackjack_bet(warn, op, host, c, now)?,
                    Response::GuestTitle if ok => self.return_to_title(op,host,c,now)?,
                    Response::SlotExit if ok => self.begin_slot_exit(now),
                    Response::BlackjackExit if ok => self.resume_at("cancel_label", op, host, c, now)?,
                    _ => {}
                }
            }
            return Ok(());
        }
        if matches!(d, FreeMode | GuestClub | Yotogi | OldYotogi) {
            self.native_menu_scroll(d, input)?;
        }
        if d == SlotMachine {
            self.tick_slot_machine(op, host, c, input, now)?;
            if self.scene != Scene::Leisure(d) || self.leisure.dialog.is_some() {
                return Ok(());
            }
        }
        if d == Examination {
            self.tick_examination(op, host, c, now)?;
        }
        if d == Photography {
            self.publish_studio_shot(op)?;
            self.studio_scroll(&input);
        }
        if d == CasinoShop && self.leisure.pressed.is_none() {
            self.casino_shop_scroll(input)?;
        }
        let hit = self.widgets.iter().enumerate().rev().find_map(|(i, w)| {
            w.collider
                .filter(|&r| business::contains(r, &input))
                .map(|_| i)
        });
        if input.buttons_pressed & buttons::MOUSE_LEFT != 0 {
            self.leisure.pressed = hit;
        }
        if released & buttons::MOUSE_LEFT == 0 || self.leisure.pressed.take() != hit {
            return Ok(());
        }
        let Some(i) = hit else { return Ok(()) };
        let path = self.widgets[i].path.clone();
        match d {
            Photography => self.studio_click(&path, op, host, c, now),
            CreativeRoom => self.room_click(&path, input, op, host, c, now),
            Examination => self.examination_click(&path, op, host, c, now),
            Casino => self.casino_click(i, &path, op, host, c, now),
            CasinoShop => self.casino_shop_click(i, &path, op, host, c, now),
            Blackjack => self.blackjack_click(i, &path, op, host, c, now),
            SlotMachine => self.slot_machine_click(&path, op, host, c, now),
            FreeMode | GuestClub | Yotogi | OldYotogi => self.native_menu_click(d, &path, op, host, c, now),
        }
    }
    fn leisure_actor(&mut self, id: usize, op: &mut Opener, now: u64) -> Result<()> {
        self.leisure_activate_actor(id, op, now)?;
        self.leisure_motion("h_kaiwa_tati_001.ks", "*会話立ち待機", op, now)
    }
    fn leisure_activate_actor(&mut self, id: usize, op: &mut Opener, now: u64) -> Result<()> {
        {
            let mut vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let s = &mut vm.runtime.host_mut().state;
            s.active_maids.resize(s.active_maids.len().max(1), None);
            s.active_maids[0] = Some(id);
        }
        self.actors
            .activate_stock(&self.script, id, 0, Some(true))?;
        self.actors.apply(
            crate::kag::AdvUi::AllProcProp {
                man: false,
                slot: 0,
            },
            &self.script,
            &self.files,
            &mut self.assets,
            op,
            now,
        )
    }
    fn leisure_motion(&mut self, file: &str, label: &str, op: &mut Opener, now: u64) -> Result<()> {
        let effects = self.actors.start_motion(
            &self.script,
            0,
            file,
            label,
            Some(0),
            None,
            true,
            &self.files,
            op,
            now,
        )?;
        self.edit_motion_effects(effects, op, now)
    }
    pub(super) fn replace_leisure_model(
        &mut self,
        name: &str,
        (glb, count): (Vec<u8>, usize),
        host: &mut dyn HostServices,
        c: ClientId,
    ) -> Result<()> {
        self.leisure.model_generation ^= 1;
        let key = 0x50000001 + self.leisure.model_generation;
        let asset = host.upload_model(c, key, &glb)?;
        let mut commands = Vec::new();
        if self.club.is_some() {
            commands.push(json!({"op":"despawn","id":1}));
        }
        commands.push(json!({"op":"spawn","entity":{"id":1,"name":name,"geometry":{"kind":"scene","asset":asset.relative()}}}));
        host.submit_engine(c, 1, &serde_json::to_vec(&json!({"world":commands}))?)?;
        if let Some(old) = self.club.replace(asset.relative().into())
            && old != asset.relative()
        {
            host.release_model(
                c,
                &crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, &old)?,
            )?;
        }
        self.club_meshes = count;
        self.script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?
            .runtime
            .host_mut()
            .state
            .bg_name = name.into();
        Ok(())
    }
}
#[cfg(test)]
mod tests;
