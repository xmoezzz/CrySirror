// SPDX-License-Identifier: MPL-2.0
//! Short installed-data fixtures through the original KAG menu entries.
use super::*;

#[test]
#[ignore = "uses the installed standalone 1.0.4 guest scene, data, models and motion"]
fn installed_guest_man_profile_and_model() -> Result<()> {
    let mut menu = Menu::with_motion(true)?;
    menu.rt.host.text_input.set_presenter_available(true);
    menu.rt.host.text_input.focus(true);
    let destination = leisure::Destination::GuestClub;
    let player_name = "Fixture Player";
    let profiles = {
        let mut vm = menu.game.script.tjs.lock().unwrap();
        let state = &mut vm.runtime.host_mut().state;
        state.player.player_name = player_name.into();
        crate::tjs::retail::guest::man_profiles(state)?
    };
    menu.game.enter(Scene::Leisure(destination), &mut menu.op, &mut menu.rt.host, menu.client, 0)?;
    menu.tick(500)?;
    let man_select = menu.game.widgets.iter()
        .find(|w| w.path.ends_with("/男選択") && w.collider.is_some())
        .and_then(|w| w.collider).context("native man select button")?;
    menu.click(man_select)?;

    let npc = profiles.iter().find(|p| p.id != crate::tjs::retail::guest::PROTAGONIST)
        .context("enabled man other than protagonist")?;
    let man_row = |menu: &Menu, id: i32| {
        let i = profiles.iter().position(|p| p.id == id).unwrap();
        let prefix = format!("native-row/{i}/");
        menu.game.widgets.iter().rev().find(|w| w.path.starts_with(&prefix) && w.collider.is_some())
            .and_then(|w| w.collider).context("native man row")
    };
    menu.click(man_row(&menu, npc.id)?)?;
    let label = |menu: &Menu, suffix: &str| {
        menu.game.widgets.iter().find(|w| w.path.ends_with(suffix))
            .and_then(|w| w.label.as_ref()).map(|l| l.text.clone()).context("native man label")
    };
    assert_eq!(label(&menu, "/Text Free Comment")?, npc.comment);
    assert_eq!(label(&menu, "/Favorite/Value")?, npc.favorite);
    assert_eq!(crate::tjs::retail::guest::man(&menu.game.script.tjs.lock().unwrap().runtime.host().state), npc.id);
    let actor = menu.game.actors.actor(true, 0).context("guest actor")?;
    assert!(actor.npc && actor.visible && !actor.chinko_visible);
    assert!(!actor.name_first.is_empty());

    menu.click(man_row(&menu, crate::tjs::retail::guest::PROTAGONIST)?)?;
    let actor = menu.game.actors.actor(true, 0).context("protagonist actor")?;
    assert!(!actor.npc && actor.visible);
    assert_eq!(actor.name_last, player_name);

    let hit = |menu: &Menu, suffix: &str| {
        menu.game.widgets.iter().rev().find(|w| w.path.ends_with(suffix) && w.collider.is_some())
            .and_then(|w| w.collider).with_context(|| format!("guest hit target {suffix}"))
    };
    menu.click(hit(&menu, "/Window Select Man/OK")?)?;
    menu.click(hit(&menu, "/ヒロイン")?)?;
    let maid_row = menu.game.widgets.iter().rev()
        .find(|w| w.path.starts_with("native-row/0/") && w.collider.is_some())
        .and_then(|w| w.collider).context("guest character row")?;
    menu.click(maid_row)?;
    let (height, maid) = {
        let vm = menu.game.script.tjs.lock().unwrap();
        let state = &vm.runtime.host().state;
        let id = crate::tjs::retail::guest::eligible_maids(state)[0];
        (state.maids[&id].body.height, id)
    };
    assert_eq!(label(&menu, "/Height/Value")?, height.to_string());
    menu.click(hit(&menu, "/LockPlayCount/仕事回数")?)?;
    menu.click(hit(&menu, "/FreeCommentWindow/FreeComment")?)?;
    assert!(menu.rt.host.text_input.commit("稽古の記録")?);
    menu.tick(16)?;
    assert_eq!(label(&menu, "/FreeComment/TextBox")?, "稽古の記録");
    let mut vm = menu.game.script.tjs.lock().unwrap();
    let status = crate::tjs::retail::guest::overview(&mut vm.runtime.host_mut().state, maid)?;
    assert!(status.work_locked && status.affinity_locked);
    assert_eq!(status.comment, "稽古の記録");
    Ok(())
}

#[test]
#[ignore = "uses the installed standalone AllMain title and return labels"]
fn installed_scene_to_title_restores_checkpoint() -> Result<()> {
    let mut menu = Menu::new()?;
    menu.open_script("allmain.ks", "*title", Scene::Title)?;
    let before = menu.game.script.tjs.lock().unwrap().runtime.host().state.player.money;
    menu.game.script.tjs.lock().unwrap().runtime.host_mut().state.player.money = before + 123;
    menu.open_script("allmain.ks", "*スタッフロール終了", Scene::Title)?;
    let restored = menu.game.script.tjs.lock().unwrap().runtime.host().state.player.money;
    assert_eq!(restored, before);
    assert_eq!(menu.game.script.tag_backup.get("label").map(String::as_str), Some("*title_end"));
    Ok(())
}

#[test]
#[ignore = "uses installed standalone game data and temporary native slots"]
fn installed_native_slot_saves_and_restores_the_daily_menu() -> Result<()> {
    use crate::save::{save_path, slot_1040::Slot};
    let mut menu = Menu::new()?;
    let daily = Scene::Daily(daily::Period::Daytime);
    menu.open(false, "*昼メニュー", daily)?;
    let (employee_guid, day) = {
        let mut tjs = menu.game.script.tjs.lock().unwrap();
        let state = &mut tjs.runtime.host_mut().state;
        state.player.money = 9_876_543_210;
        state.retail.casino_coins = 751;
        let guid = state.maids[&state.stock[0]].guid_id.clone();
        state.player.schedule_slot[0].maid_guid.clone_from(&guid);
        let icon = crysirror_core::image_processing::RgbaImage::new(
            menu.client,
            [90, 90],
            [200, 30, 70, 255].repeat(90 * 90).into(),
        )?;
        state.retail.set_stock_thumbnail(&guid, std::sync::Arc::new(icon))?;
        state.retail.casino_sold_out.insert(1000);
        (guid, state.player.days)
    };
    let click_slot_control = |menu: &mut Menu, suffix: &str| -> Result<()> {
        let rect = menu.game.widgets.iter().rev()
            .find(|w| w.path.ends_with(suffix) && w.collider.is_some())
            .and_then(|w| w.collider)
            .with_context(|| format!("missing slot control {suffix}"))?;
        menu.click(rect)
    };
    menu.game.open_slots(slots::Mode::DailySave, &mut menu.op, &mut menu.rt.host, menu.client, 0)?;
    click_slot_control(&mut menu, "serialNo_0/EmptyUnit")?;
    click_slot_control(&mut menu, "SystemDialog/Base/Ok")?;
    let path = save_path(0)?;
    let bytes = menu.op.storage.read(&path).context("menu did not create the slot")?;
    let saved = Slot::read(&bytes)?;
    assert_eq!(saved.roster.men.len(), 6);
    assert_eq!(saved.roster.employees[0].status.guid, employee_guid);
    assert_eq!(saved.roster.player.days, day);
    assert_eq!(saved.roster.player.money, 9_876_543_210);
    assert_eq!(saved.roster.player.coins, 751);
    assert_eq!(saved.roster.player.slots[0].guid, employee_guid);
    assert_eq!(saved.roster.player.backups.len(), 2);
    assert!(saved.casino.sold_out.contains(&(1000, true)));
    assert!(saved.roster.employees[0].misc.thumbnail.starts_with(b"\x89PNG\r\n\x1a\n"));

    // The ordinary title load button uses this same panel. Use the title
    // route to ensure load does not depend on retaining the old scene.
    menu.game.slot_viewer = None;
    menu.title()?;
    {
        let mut tjs = menu.game.script.tjs.lock().unwrap();
        let state = &mut tjs.runtime.host_mut().state;
        state.player.money = 3;
        state.retail.casino_coins = 0;
        state.maids.clear();
        state.stock.clear();
    }
    menu.game.open_slots(slots::Mode::TitleLoad, &mut menu.op, &mut menu.rt.host, menu.client, 0)?;
    click_slot_control(&mut menu, "serialNo_0/DataUnit")?;
    click_slot_control(&mut menu, "SystemDialog/Base/Ok")?;
    if let Some(viewer) = &menu.game.slot_viewer {
        let errors: Vec<_> = viewer.dialog.as_ref().map(|dialog| {
            menu.game.widgets[dialog.first..].iter().filter_map(|w| w.label.as_ref().map(|l| l.text.clone())).collect()
        }).unwrap_or_default();
        bail!("native slot did not restore: {errors:?}");
    }
    menu.wait(daily)?;
    {
        let tjs = menu.game.script.tjs.lock().unwrap();
        let state = &tjs.runtime.host().state;
        assert_eq!(state.player.money, 9_876_543_210);
        assert_eq!(state.retail.casino_coins, 751);
        assert!(state.retail.casino_sold_out.contains(&1000));
        assert_eq!(state.player.schedule_slot[0].maid_guid, employee_guid);
        let id = state.stock[0];
        assert_eq!(state.maids[&id].guid_id, employee_guid);
        assert!(state.retail.stock_models.contains_key(&id));
        let icon = crate::tjs::retail::operations::portrait(state, id).context("restored stock portrait")?;
        assert_eq!(icon.size(), [90, 90]);
        assert_eq!(icon.pixel_at([2, 3])?, Some([200, 30, 70, 255]));
    }
    assert_eq!(menu.op.storage.read(&path)?, bytes);
    let incomplete = save_path(1)?;
    menu.op.storage.write_atomic(&incomplete, &bytes[..bytes.len() - 1])?;
    let scene = menu.game.scene;
    assert!(menu.game.load_native_slot(&mut menu.op, &mut menu.rt.host, menu.client, 1, 0).is_err());
    assert_eq!(menu.game.scene, scene);
    assert_eq!(menu.game.script.tjs.lock().unwrap().runtime.host().state.player.money, 9_876_543_210);
    Ok(())
}

#[test]
#[ignore = "uses installed standalone guest menus, scripts, models and motion"]
fn installed_title_guest_and_club_slots_round_trip() -> Result<()> {
    use crate::{
        save::{save_path, slot_1040::{edit_guest_slots, read_guest_slots}},
        tjs::retail::guest,
    };
    let mut menu = Menu::with_motion(true)?;
    let club = Scene::Leisure(leisure::Destination::GuestClub);
    menu.open(false, "*昼メニュー", Scene::Daily(daily::Period::Daytime))?;
    menu.game.write_native_slot(&menu.op, 13, "本編".into())?;
    let path = save_path(13)?;
    let original = menu.op.storage.read(&path)?;
    let other = save_path(14)?;
    menu.op.storage.write_atomic(&other, &original)?;
    let click = |menu: &mut Menu, suffix: &str| -> Result<()> {
        let rect = menu.game.widgets.iter().rev()
            .find(|w| w.path.ends_with(suffix) && w.collider.is_some())
            .and_then(|w| w.collider)
            .with_context(|| format!("missing guest slot button {suffix}"))?;
        menu.click(rect)
    };
    let enter_origin = |menu: &mut Menu| -> Result<()> {
        click(menu, "/GuestGroup/Guest")?;
        click(menu, "serialNo_13/DataUnit")?;
        click(menu, "SystemDialog/Base/Ok")?;
        if !menu.game.widgets.iter().any(|w| w.path.ends_with("serialNo_0/DataUnit")) {
            let messages: Vec<_> = menu.game.widgets.iter().filter_map(|w| w.label.as_ref())
                .map(|l| l.text.as_str()).filter(|text| text.contains("失敗")).collect();
            bail!("guest local panel was not opened: {messages:?}");
        }
        Ok(())
    };
    menu.open_script("allmain.ks", "*title", Scene::Title)?;
    menu.tick(3100)?;
    enter_origin(&mut menu)?;
    assert!(!menu.game.widgets.iter().any(|w| w.path.ends_with("/DisplaySave")));
    click(&mut menu, "/Page_2")?;
    assert!(menu.game.widgets.iter().any(|w| w.path.ends_with("serialNo_16/DataUnit")));
    click(&mut menu, "SaveAndLoadPanel/Cancel")?;
    menu.tick(550)?;
    click(&mut menu, "serialNo_13/DataUnit")?;
    click(&mut menu, "SystemDialog/Base/Ok")?;
    click(&mut menu, "serialNo_0/DataUnit")?;
    click(&mut menu, "SystemDialog/Base/Ok")?;
    menu.wait(club)?;
    menu.tick(500)?;
    {
        let mut script = menu.game.script.tjs.lock().unwrap();
        let state = &mut script.runtime.host_mut().state;
        assert_eq!(guest::origin(state)?, 13);
        assert_eq!(state.player.flag("傅きモード"), 1);
        let employee = state.stock[0];
        guest::set_comment(state, employee, "会所セーブ".into())?;
        guest::toggle_parameter_lock(state, false);
    }
    click(&mut menu, "/セーブ")?;
    click(&mut menu, "serialNo_1/DataUnit")?;
    click(&mut menu, "SystemDialog/Base/Ok")?;
    let revised = menu.op.storage.read(&path)?;
    let records = read_guest_slots(&revised)?;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].slot, 1);
    let campaign = edit_guest_slots(&revised, |records| {
        records.clear();
        Ok(())
    })?;
    assert_eq!(campaign, original);
    assert_eq!(menu.op.storage.read(&other)?, original);

    click(&mut menu, "SystemDialog/Base/Ok")?;
    click(&mut menu, "SaveAndLoadPanel/Cancel")?;
    menu.tick(550)?;
    {
        let mut script = menu.game.script.tjs.lock().unwrap();
        let state = &mut script.runtime.host_mut().state;
        let employee = state.stock[0];
        guest::set_comment(state, employee, "未保存の変更".into())?;
        guest::toggle_parameter_lock(state, false);
    }
    click(&mut menu, "/ロード")?;
    click(&mut menu, "serialNo_1/DataUnit")?;
    click(&mut menu, "SystemDialog/Base/Ok")?;
    let verify = |menu: &Menu| -> Result<()> {
        let mut script = menu.game.script.tjs.lock().unwrap();
        let state = &mut script.runtime.host_mut().state;
        let employee = state.stock[0];
        let info = guest::overview(state, employee)?;
        assert_eq!(info.comment, "会所セーブ");
        assert!(info.work_locked);
        assert_eq!(guest::origin(state)?, 13);
        assert_eq!(state.player.flag("傅きモード"), 1);
        Ok(())
    };
    verify(&menu)?;
    assert!(menu.game.slot_viewer.is_none());

    menu.game.return_to_title(&mut menu.op, &mut menu.rt.host, menu.client, 0)?;
    menu.wait(Scene::Title)?;
    menu.tick(3100)?;
    enter_origin(&mut menu)?;
    click(&mut menu, "serialNo_1/DataUnit")?;
    click(&mut menu, "SystemDialog/Base/Ok")?;
    menu.wait(club)?;
    verify(&menu)?;
    assert_eq!(menu.op.storage.read(&path)?, revised);
    Ok(())
}

#[test]
#[ignore = "uses the installed standalone hall model and textures"]
fn installed_story_entrance_lighting() -> Result<()> {
    let (_dir, _, _, mut op, _, _, mut assets) = daily::installed_panel(&[])?;
    let (glb, _) = club::prefab(&mut assets.unity, &mut op, "EmpireClub_Entrance")?;
    let model = crysirror_asset::pbr::load_geometry(&glb, |path| {
        bail!("unexpected external hall asset {path}")
    })?;
    let glow = model.materials.iter().position(|m| {
        m.alpha_mode == crysirror_asset::pbr::AlphaMode::SoftAdditive
            && m.unlit
            && m.double_sided
            && m.base_color_texture.is_some()
    }).context("hall glow material")?;
    let wall = model.materials.iter().position(|m| {
        matches!(m.effect, Some(crysirror_asset::pbr::ShaderEffect::RgbLightmap { .. }))
            && m.emissive_texture.is_some()
    }).context("hall wall lightmap")?;
    assert!(model.meshes.iter().flatten().any(|p| p.material == glow));
    let receivers: Vec<_> = model.meshes.iter().flatten().filter(|p| p.material == wall).collect();
    assert!(!receivers.is_empty());
    assert!(receivers.iter().all(|p| p.aux.as_ref().is_some_and(|uv| uv.len() == p.vertices.len())));
    Ok(())
}

#[test]
#[ignore = "uses the installed standalone office and message window"]
fn installed_story_room_and_dialogue_layout() -> Result<()> {
    let (_dir, mut rt, c, mut op, _, _, mut assets) = daily::installed_panel(&[])?;
    let (glb, _) = club::prefab(&mut assets.unity, &mut op, "ShinShitsumu")?;
    let json_len = u32::from_le_bytes(glb[12..16].try_into()?) as usize;
    let doc: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len])?;
    let mats = doc["materials"].as_array().context("office materials")?;
    for (name, unlit, alpha) in [
        ("ShinShitsumu_Monitor", true, "OPAQUE"),
        ("ShinShitsumu_Light_alpha", true, "BLEND"),
        ("ShinShitsumu_01", false, "OPAQUE"),
    ] {
        let mat = mats.iter().find(|m| m["name"] == name).context(name)?;
        assert_eq!(mat["alphaMode"].as_str().unwrap_or("OPAQUE"), alpha, "{name}");
        let engine = &mat["extras"]["crysirror_material"]["material"];
        assert_eq!(engine["unlit"].as_bool().unwrap_or(false), unlit, "{name}");
        assert!(mat["pbrMetallicRoughness"]["baseColorTexture"]["index"].is_number());
    }
    let mut panel = assets.scene(&mut op, &mut rt.host, c, "_cm3d21")?;
    let body = panel.iter_mut().find(|w| w.path.ends_with("/MsgParent/Message")).context("message")?;
    let mut rect = body.rect;
    let label = body.label.as_mut().context("dialogue label")?;
    label.set_text("第一行\n第二行\n第三行\n第四行");
    let timeout = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while label.layers(&mut rt.host, c, rect, [255; 4])?.is_empty() {
        ensure!(std::time::Instant::now() < timeout, "dialogue font did not load");
        rt.tick_control(16, InputFrame::default())?;
        std::thread::yield_now();
    }
    let lines = label.layers(&mut rt.host, c, rect, [255; 4])?;
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[2].1["text"], "第三行");
    assert_eq!(lines[2].0[1] - lines[0].0[1], 50.);
    let x = lines[0].0[0];
    rect[0] += 40.;
    rect[3] = 25.;
    let lines = label.layers(&mut rt.host, c, rect, [128; 4])?;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].0[0], x + 40.);
    assert_eq!(lines[0].1["color"], json!([128, 128, 128, 128]));
    Ok(())
}

pub(super) struct Menu {
    _dir: tempfile::TempDir,
    pub(super) rt: crysirror_runtime::Runtime,
    pub game: Game,
    pub(in crate::com3d2) op: Opener,
    pub(super) client: ClientId,
}
impl Menu {
    pub fn new() -> Result<Self> {
        Self::with_motion(false)
    }
    pub fn with_motion(motion:bool)->Result<Self>{
        let mut packs=vec!["sound", "sound2", "parts", "parts2"];
        if motion{packs.push("motion2");}
        let (dir, mut rt, client, op, _, _, _) = daily::installed_panel(&packs)?;
        if motion{mount_motion(&mut rt,client,&dir)?;packs.push("motion");}
        let mut vm = Vm {
            linked_cm3d2: None,
            client,
            config: Config {
                data_dir: op.dir.clone(),
                archives: ["csv", "system", "system2", "script"]
                    .into_iter()
                    .chain(packs)
                    .map(str::to_owned)
                    .collect(),
            },
            game: None,
        };
        vm.advance(&mut rt.host, InputFrame::default())?;
        let game = vm.game.take().unwrap();
        {
            let mut vm = game.script.tjs.lock().unwrap();
            let s = &mut vm.runtime.host_mut().state;
            s.player.schedule_slot.resize_with(40, Default::default);
            crate::tjs::retail::operations::start_day(s)?;
            vm.eval("CreateNewMaid('Muku')")?;
            crate::tjs::retail::finish_profile(&mut vm.runtime.host_mut().state, true)?;
        }
        Ok(Self {
            _dir: dir,
            rt,
            client,
            op,
            game,
        })
    }
    pub fn title(&mut self) -> Result<()> {
        self.game.enter(
            Scene::Title,
            &mut self.op,
            &mut self.rt.host,
            self.client,
            0,
        )?;
        self.tick(3100)
    }
    pub fn open(&mut self, night: bool, label: &str, scene: Scene) -> Result<()> {
        let file = if night {
            "nightmain.ks"
        } else {
            "daytimemain.ks"
        };
        self.game
            .script
            .tjs
            .lock()
            .unwrap()
            .runtime
            .host_mut()
            .state
            .player
            .generic_flag
            .insert("時間帯".into(), if night { 3 } else { 2 });
        self.open_script(file, label, scene)
    }
    pub fn open_script(&mut self, file: &str, label: &str, scene: Scene) -> Result<()> {
        self.game
            .script
            .load(&self.game.files, self.op.storage.as_ref(), file, label)?;
        self.game.script.enabled = true;
        self.game.script.exec();
        self.game.enter(
            Scene::Script,
            &mut self.op,
            &mut self.rt.host,
            self.client,
            0,
        )?;
        self.wait(scene)?;
        self.tick(500)
    }
    pub fn wait(&mut self, scene: Scene) -> Result<()> {
        for _ in 0..48 {
            if self.game.scene == scene {
                return Ok(());
            }
            self.tick(250)?;
        }
        bail!(
            "expected {scene:?}, reached {:?} at {}",
            self.game.scene,
            self.game.script.position()
        )
    }
    pub fn tick(&mut self, ms: u64) -> Result<()> {
        self.rt.tick_control(ms, InputFrame::default())?;
        self.game.update(
            &mut self.rt.host,
            self.client,
            &mut self.op,
            InputFrame::default(),
        )
    }
    pub fn click(&mut self, r: [f32; 4]) -> Result<()> {
        let input = InputFrame {
            buttons_down: buttons::MOUSE_LEFT,
            buttons_pressed: buttons::MOUSE_LEFT,
            pointer_x: (r[0] + r[2] / 2.) as i32,
            pointer_y: (r[1] + r[3] / 2.) as i32,
            ..Default::default()
        };
        self.game
            .update(&mut self.rt.host, self.client, &mut self.op, input)?;
        self.game.update(
            &mut self.rt.host,
            self.client,
            &mut self.op,
            InputFrame {
                buttons_pressed: 0,
                buttons_down: 0,
                ..input
            },
        )
    }
}

pub(in crate::com3d2) fn mount_motion(rt:&mut crysirror_runtime::Runtime,c:ClientId,tmp:&tempfile::TempDir)->Result<()>{
    use crysirror_core::path::{Mount,VfsPath};
    use crysirror_resource::{ArchiveMount,DecodeContext,FormatRegistry};
    use std::{path::PathBuf,sync::Arc};
    let archive = std::env::var_os("COM3D2_MOTION_ARCHIVE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("COM3D2_ROOT").unwrap()).join("GameData/motion.arc")
        });
    let mut disk = crysirror_runtime::vfs::DiskVfs::new(
        archive.parent().context("motion archive parent")?,
        tmp.path(),
    )?;
    disk.register(c, "com3d2", "motion")?;
    let source = disk.storage(c)?.open_asset(&VfsPath::new(
        Mount::Asset,
        archive
            .file_name()
            .and_then(|v| v.to_str())
            .context("motion archive name")?,
    )?)?;
    let cx = DecodeContext::new(c);
    let mut reg = FormatRegistry::default();
    crysirror_formats_arc::register(&mut reg)?;
    let (_, arc) = reg.open(source, Some("kiss/warc"), &cx)?;
    rt.mount_provider(c, "motion", Arc::new(ArchiveMount::new(arc, cx)?))?;
    Ok(())
}
