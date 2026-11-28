// SPDX-License-Identifier: MPL-2.0
//! Standalone GameMain slot capture, preparation and scene handover.
use super::*;
use crate::{
    save::{save_path, slot_1040::Slot},
    tjs::{
        ScriptProfile,
        retail::{guest, native_save},
    },
};
use crysirror_core::path::{Mount, VfsPath};

struct PreparedLoad {
    script: AdvKag,
    actors: actors::Actors,
    widgets: Vec<Widget>,
}

impl Game {
    pub(super) fn write_native_slot(
        &mut self,
        op: &Opener,
        slot: i32,
        comment: String,
    ) -> Result<()> {
        ensure!(
            matches!(self.scene, Scene::Daily(_)),
            "native save requires a daily menu checkpoint"
        );
        let checkpoint = self.script.serialize_blob()?;
        let mut state = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?
            .runtime
            .host()
            .state
            .clone();
        self.desk.catalog(&mut state)?;
        let bytes = native_save::capture(
            &mut state,
            &checkpoint,
            self.script.fade_wait(),
            self.script.enabled,
            self.actors.native_men(),
            comment,
        )?
        .encode()?;
        op.storage.write_atomic(&save_path(slot)?, &bytes)?;
        Ok(())
    }

    pub(super) fn load_native_slot(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        slot: i32,
        now: u64,
    ) -> Result<()> {
        self.load_native_slot_as(op, host, client, slot, now, None)
    }

    pub(super) fn load_native_guest(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        origin: i32,
        guest_slot: Option<i32>,
        now: u64,
    ) -> Result<()> {
        self.load_native_slot_as(op, host, client, origin, now, Some(guest_slot))
    }

    fn load_native_slot_as(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        slot: i32,
        now: u64,
        guest_slot: Option<Option<i32>>,
    ) -> Result<()> {
        let bytes = op.storage.read(&save_path(slot)?)?;
        let staged = self.prepare_native_load(op, host, client, &bytes, slot, guest_slot)?;
        // All parsing, game-table reads, menu/model reconstruction and
        // dependency preparation above leave the current session intact.
        self.actors.leave(host, client)?;
        self.sound.stop_all(host, client)?;
        if let Some(asset) = &self.club {
            let asset = VfsPath::new(Mount::Asset, asset)?;
            host.submit_engine(client, 1, br#"{"world":[{"op":"despawn","id":1}]}"#)?;
            host.release_model(client, &asset)?;
        }
        self.club = None;
        self.club_meshes = 0;
        self.script = staged.script;
        self.actors = staged.actors;
        self.widgets = staged.widgets;
        self.slot_viewer = None;
        self.scene = Scene::Script;
        self.entered = now;
        self.leaving = None;
        self.pressed = None;
        self.pointer = None;
        self.config_panel = None;
        self.settings.control_press = None;
        self.settings.shortcuts_open = false;
        self.settings.shortcut_base = false;
        self.dialogue = None;
        self.voice_maid = None;
        self.title_banner = None;
        self.choices = None;
        self.message_open = false;
        self.adv_cover = None;
        self.world_cover = None;
        self.camera_move = None;
        self.day_camera_fade = None;
        self.main_light = Default::default();
        self.trophies = Default::default();
        self.leisure = Default::default();
        self.desk = Default::default();
        Ok(())
    }

    fn prepare_native_load(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        bytes: &[u8],
        origin: i32,
        guest_slot: Option<Option<i32>>,
    ) -> Result<PreparedLoad> {
        let slot = Slot::read(bytes)?;
        let (mut state, men) = {
            let old = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            native_save::stage(&old.runtime.host().state, &slot, client)?
        };
        if let Some(index) = guest_slot {
            let record = index
                .map(|index| {
                    slot.guest_slots
                        .iter()
                        .find(|guest| guest.slot == index)
                        .context("missing guest checkpoint")
                })
                .transpose()?;
            guest::apply_slot(&mut state, origin, record)?;
        }
        let mut desk = desk::Desk::default();
        desk.catalog(&mut state)?;
        let mut script = AdvKag::for_profile(
            &self.files,
            op.storage.as_ref(),
            ScriptProfile::Com3d2,
            state,
        )?;
        script.deserialize_disk_blob(
            &self.files,
            op.storage.as_ref(),
            slot.management.script.checkpoint,
        )?;
        ensure!(
            script.current_storage().is_some(),
            "native slot has no restorable script"
        );
        script.set_fade_wait(slot.management.script.fade_wait);
        script.enabled = slot.management.script.enabled;
        script.exec();
        let actors = actors::Actors::restore_native_roster(&script, men)?;
        let mut widgets = self.assets.scene(op, host, client, "SceneADV")?;
        widgets.extend(self.assets.scene(op, host, client, "_cm3d21")?);
        widgets.extend(self.assets.scene(op, host, client, "StandaloneShortcuts")?);
        Ok(PreparedLoad {
            script,
            actors,
            widgets,
        })
    }
}
