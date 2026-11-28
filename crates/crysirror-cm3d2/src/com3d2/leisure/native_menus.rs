// SPDX-License-Identifier: MPL-2.0
//! Standalone 1.0.4 recollection, guest and stage-selection controllers.
use super::*;
use crate::{
    nei::Table,
    tjs::{TjsState, retail::guest},
};

mod guest_replay;
mod guest_profile;
mod sub_cast;
mod free_skill;
mod vip;
pub(in crate::com3d2) use vip::Layout as VipLayout;
pub(in crate::com3d2) use free_skill::Layout as FreeSkillLayout;
pub(in crate::com3d2) use free_skill::is_free_skill;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Main,
    Story,
    Everyday,
    Vip,
    GuestMan,
    GuestRooms,
    GuestRoomMaid,
    GuestHeroine,
    GuestFree,
    GuestReplay,
    Stage,
    SubCast,
    FreeSkill,
}

impl Page {
    fn guest_character_picker(self) -> bool {
        matches!(self, Self::GuestHeroine | Self::GuestRoomMaid | Self::GuestFree)
    }

    fn prefix(self, destination: Destination) -> &'static str {
        match self {
            Self::Main if destination == FreeMode => "UI Root/MenuSelect/",
            Self::Main => "UI Root/Window Main Menu/",
            Self::Story => "UI Root/MenuStory/",
            Self::Everyday => "UI Root/MenuEveryday/",
            Self::Vip => "UI Root/MenuVip/",
            Self::GuestMan => "UI Root/Window Select Man/",
            Self::GuestRooms => "UI Root/Window Select Club/",
            Self::GuestRoomMaid | Self::GuestFree | Self::GuestHeroine => {
                "UI Root/MaidMiniManagement/"
            }
            Self::GuestReplay => guest_replay::PREFIX,
            Self::Stage => "UI Root/StageSelectPanel/",
            Self::SubCast => sub_cast::PREFIX,
            Self::FreeSkill => free_skill::PREFIX,
        }
    }
}

#[derive(Clone)]
struct Row {
    title: String,
    detail: String,
    key: String,
    enabled: bool,
}

struct StagePreview {
    backdrop: String,
    music: String,
    camera: Orbit,
    thumbnail: String,
    asset: Option<String>,
}

#[derive(Default)]
pub(super) struct Menu {
    page: Page,
    story: Vec<Row>,
    everyday: Vec<Row>,
    vip: Option<vip::Screen>,
    rows: Vec<Row>,
    template: Vec<Widget>,
    selected: Option<usize>,
    offset: usize,
    day: bool,
    guest_men: Vec<guest::ManProfile>,
    guest_man: Option<i32>,
    guest_comment: Option<guest_profile::Comment>,
    guest_room: Option<(i32, i32)>,
    replay: guest_replay::Replay,
    sub_cast: sub_cast::Picker,
    free_skill: Option<free_skill::Picker>,
    stages: std::collections::BTreeMap<String, StagePreview>,
}

impl Menu {
    fn selected_guest_maid(&self) -> Result<Option<usize>> {
        if !self.page.guest_character_picker() {
            return Ok(None);
        }
        self.selected.and_then(|i| self.rows.get(i)).filter(|row| row.enabled)
            .map(|row| row.key.parse().context("guest character id")).transpose()
    }
}

fn camera(text: &str) -> Result<Orbit> {
    let values = text
        .split(',')
        .map(|v| v.trim().parse::<f32>())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let [x, y, z, distance, yaw, pitch]: [f32; 6] = values
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid stage camera"))?;
    ensure!(
        [x, y, z, distance, yaw, pitch]
            .into_iter()
            .all(f32::is_finite)
            && distance > 0.,
        "invalid stage camera"
    );
    Ok(([x, y, z], distance, [yaw, pitch]))
}

fn row_id(t: &Table, id: i32) -> Option<usize> {
    (1..t.rows).find(|&r| t.exists(0, r) && t.int(0, r) == id)
}

fn guest_roster(s: &TjsState, ids: Vec<usize>) -> Vec<Row> {
    ids.into_iter()
        .filter_map(|id| {
            let m = s.maids.get(&id)?;
            Some(Row {
                title: format!("{} {}", m.last_name, m.first_name),
                detail: m.profile.clone(),
                key: id.to_string(),
                enabled: true,
            })
        })
        .collect()
}

fn guest_rooms(s: &mut TjsState, man: i32) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    for room in guest::rooms(s, man, true)? {
        let playable = !guest::room_roster(s, room.id, room.man)?.is_empty();
        rows.push(Row {
            title: room.name,
            detail: room.description,
            key: room.id.to_string(),
            enabled: playable,
        });
    }
    Ok(rows)
}

fn row_list(s: &mut TjsState, story: bool) -> Result<Vec<Row>> {
    Ok(crate::tjs::retail::recollection::scenes(s, story)?.into_iter().map(|row| Row {
        title: row.title, detail: row.description, key: row.file, enabled: row.enabled,
    }).collect())
}

impl Game {
    pub(super) fn open_native_menu(
        &mut self,
        d: Destination,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<String> {
        use Page::*;
        if is_free_skill(d, &self.script.tag_backup) {
            return self.open_free_skill(d, op, h, c, now);
        }
        if matches!(d, Yotogi | OldYotogi)
            && self
                .script
                .tag_backup
                .get("start")
                .is_some_and(|start| start == "SubCharaSelect")
        {
            return self.open_sub_cast(d, op, h, c);
        }
        let mut vm = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let s = &mut vm.runtime.host_mut().state;
        let menu = &mut self.leisure.native_menu;
        let (backdrop, music, camera) = match d {
            FreeMode => {
                menu.story = row_list(s, true)?;
                menu.everyday = row_list(s, false)?;
                let personal = s
                    .maid(0)
                    .map(|m| m.personal)
                    .context("recollection requires an active character")?;
                let t = s.retail.table("maid_status_personal_list.nei")?;
                let id = row_id(&t, personal).context("recollection character personality")?;
                let bgm = t.string(4, id);
                let bgm = (!bgm.is_empty()).then(|| crate::sound::ogg(&bgm));
                (
                    "Theater".into(),
                    bgm.filter(|f| self.files.exists(f)),
                    ([0., 1.2, 0.], 3.4, [180., 6.]),
                )
            }
            GuestClub => {
                if self.script.tag_backup.contains_key("update_maid_status") {
                    guest::post_scenario(s);
                }
                menu.guest_man = Some(guest::man(s));
                menu.guest_men = guest::man_profiles(s)?;
                (
                    "EmpireClub_Entrance".into(),
                    Some("BGM020.ogg".to_owned()),
                    ([0., 5.5, 11.], 1.4, [180., 3.]),
                )
            }
            Yotogi | OldYotogi => {
                // The native Null screen hands the stored label directly back to
                // ADV. It is handled before loading the stage/skill controllers.
                menu.page = match self.script.tag_backup.get("start").map(String::as_str) {
                    Some("StageSelect") => Stage,
                    None => Stage,
                    Some(other) => bail!("COM3D2 {} controller {other} is not attached", d.name()),
                };
                if d == OldYotogi {
                    for stage in crate::tjs::retail::linked::stages(s)? {
                        menu.stages.insert(
                            stage.key.clone(),
                            StagePreview {
                                backdrop: stage.backdrop,
                                music: stage.music,
                                camera: camera(&stage.camera)?,
                                thumbnail: stage.thumbnail,
                                asset: None,
                            },
                        );
                        menu.rows.push(Row {
                            title: stage.name,
                            detail: String::new(),
                            key: stage.key,
                            enabled: stage.enabled,
                        });
                    }
                } else {
                    let table = s.retail.table("yotogi_stage_list.nei")?;
                    menu.day = s.player.flag("時間帯") == 2;
                    let mut sorted = Vec::new();
                    for row in 1..table.rows {
                        if !table.exists(0, row)
                            || !s
                                .retail
                                .enabled("yotogi_stage_enabled_list", table.int(0, row))?
                        {
                            continue;
                        }
                        let key = table.string(1, row);
                        let column = if menu.day { 7 } else { 9 };
                        menu.stages.insert(
                            key.clone(),
                            StagePreview {
                                backdrop: table.string(column, row),
                                thumbnail: table.string(column + 1, row),
                                music: table.string(11, row),
                                camera: camera(&table.string(12, row))?,
                                asset: None,
                            },
                        );
                        let facility_ids = table.string(6, row);
                        let playable = facility_ids.trim().is_empty()
                            || facility_ids.split(',').any(|id| {
                                id.trim()
                                    .parse()
                                    .is_ok_and(|id| s.retail.has_built_facility(id))
                            });
                        sorted.push((
                            table.int(3, row),
                            Row {
                                title: table.string(2, row),
                                detail: String::new(),
                                key,
                                enabled: s.player.current_salon_grade >= table.int(5, row)
                                    && playable,
                            },
                        ));
                    }
                    sorted.sort_by_key(|r| r.0);
                    menu.rows = sorted.into_iter().map(|r| r.1).collect();
                }
                menu.selected = menu.rows.iter().position(|row| row.enabled);
                let first = menu
                    .selected
                    .and_then(|i| menu.rows.get(i))
                    .or_else(|| menu.rows.first())
                    .context("no stage data for the current profile")?;
                if first.enabled {
                    s.stage_name = first.key.clone();
                    s.stage_prefab = menu.stages[&first.key].backdrop.clone();
                }
                for row in &menu.rows {
                    let preview = menu
                        .stages
                        .get_mut(&row.key)
                        .context("missing stage preview")?;
                    let name = if row.enabled {
                        let file = &preview.thumbnail;
                        if file.contains('?') {
                            crate::tjs::retail::replace_script_name(s, file)
                                .transpose()?
                                .unwrap_or_else(|| file.clone())
                        } else {
                            file.clone()
                        }
                    } else {
                        "cm3d2_stageselectpreview_nowprinting.tex".to_owned()
                    };
                    let stem = name
                        .rsplit_once('.')
                        .map_or(name.as_str(), |(stem, _)| stem);
                    let fallback = "cm3d2_stageselectpreview_dammy";
                    let files = if d == OldYotogi {
                        s.retail.linked_files()
                    } else {
                        &self.files
                    };
                    let visible = if files.exists(&format!("{stem}.tex")) {
                        stem
                    } else {
                        fallback
                    };
                    if files.exists(&format!("{visible}.tex")) {
                        preview.asset = Some(self.assets.archive_icon(
                            op,
                            h,
                            c,
                            files,
                            visible,
                            d == OldYotogi,
                        )?);
                    }
                }
                let preview = &menu.stages[&first.key];
                (
                    preview.backdrop.clone(),
                    Some(crate::sound::ogg(&preview.music)),
                    preview.camera,
                )
            }
            _ => bail!("invalid native menu {}", d.name()),
        };
        let legacy = crate::tjs::retail::recollection::legacy_mode(s);
        let no_yotogi = s.maid(0).is_none_or(|m| m.yotogi_play_count == 0);
        let stock_model = s
            .active_maids
            .first()
            .copied()
            .flatten()
            .filter(|id| s.retail.stock_models.contains_key(id));
        drop(vm);
        if d == FreeMode {
            menu.template = self.assets.scene(op, h, c, "RecollectionRow")?;
            self.leisure.base.retain(|w| {
                !(legacy && (w.path.ends_with("/FreeModeMenuButton/VIP_HEvent")
                    || w.path.ends_with("/FreeModeMenuButton/ストーリー")))
                    && !(!legacy && w.path.ends_with("/FreeModeMenuButton/VIP"))
                    && !(menu.everyday.is_empty() && w.path.ends_with("/FreeModeMenuButton/日常"))
            });
            for widget in &mut self.leisure.base {
                if no_yotogi && widget.path.ends_with("/夜伽")
                    || menu.story.is_empty() && widget.path.ends_with("/ストーリー")
                {
                    widget.collider = None;
                    widget.tint[3] = widget.tint[3].min(100);
                }
            }
        } else if matches!(d, Yotogi | OldYotogi) {
            menu.template = self.assets.scene(
                op,
                h,
                c,
                if d == OldYotogi {
                    "StageSelectionOldRow"
                } else {
                    "StageSelectionRow"
                },
            )?;
        }
        if d == FreeMode
            && let Some(id) = stock_model
        {
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
            )?;
        }
        if d == GuestClub {
            let man = self.leisure.native_menu.guest_man.context("missing guest man selection")?;
            self.show_guest_man(man, op, now)?;
        }
        self.adv_camera = camera;
        self.camera_control = true;
        if let Some(music) = music {
            self.native_menu_music(d, &music, 0.5, h, c)?;
        }
        self.draw_native_menu(d)?;
        Ok(backdrop)
    }

    fn native_menu_music(
        &mut self,
        destination: Destination,
        file: &str,
        fade: f32,
        host: &mut dyn HostServices,
        client: ClientId,
    ) -> Result<()> {
        let vm = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let legacy = destination == OldYotogi;
        let state = &vm.runtime.host().state.retail;
        if let Some(files) = state.audio_files(&self.files, Kind::Bgm, file, legacy) {
            self.sound
                .play_profile(host, client, files, Kind::Bgm, file, fade, true, legacy)?;
        }
        Ok(())
    }

    fn draw_native_menu(&mut self, d: Destination) -> Result<()> {
        let menu = &self.leisure.native_menu;
        let prefix = menu.page.prefix(d);
        self.widgets = self
            .leisure
            .base
            .iter()
            .filter(|widget| {
                (widget.path.starts_with(prefix)
                    && !widget.path.contains("/temp item")
                    && !(menu.page == Page::GuestMan
                        && widget.path.contains("/Window Man Info/"))
                    && !(menu.page == Page::GuestHeroine
                        && widget.path.split('/').any(|name| name == "Cancel")))
                    || widget.path.starts_with("SystemUI Root/SystemShortcut/")
            })
            .cloned()
            .collect();
        if menu.page == Page::GuestReplay {
            return self.draw_guest_replay();
        }
        if menu.page == Page::SubCast {
            return self.draw_sub_cast();
        }
        if menu.page == Page::FreeSkill {
            return self.draw_free_skill();
        }
        if menu.page == Page::Vip {
            return self.draw_vip();
        }
        let guest_status = if let Some(id) = menu.selected_guest_maid()? {
            let mut vm = self.script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
            Some(guest::overview(&mut vm.runtime.host_mut().state, id)?)
        } else {
            None
        };
        if menu.page.guest_character_picker() {
            self.widgets.retain_mut(|w| {
                if w.path.contains("/MaidMiniStatusPanel/") {
                    if let Some(status) = &guest_status {
                        guest_profile::paint(w, status, menu.page == Page::GuestHeroine);
                    }
                    return guest_status.is_some();
                }
                if w.path.contains("/Window Parameter Lock/") {
                    let Some(status) = &guest_status else { return false };
                    if menu.page == Page::GuestFree {
                        return false;
                    }
                    let checked = if w.path.contains("/LockLikability/") {
                        if menu.guest_man == Some(guest::PROTAGONIST) {
                            w.collider = None;
                        }
                        status.affinity_locked
                    } else if w.path.contains("/LockPlayCount/") {
                        status.work_locked
                    } else {
                        return true;
                    };
                    return !w.path.ends_with("/off") && !w.path.ends_with("/on")
                        || w.path.ends_with(if checked { "/on" } else { "/off" });
                }
                true
            });
        }
        for w in &mut self.widgets {
            if menu.page == Page::GuestMan
                && let Some(man) = menu.guest_men.iter().find(|m| Some(m.id) == menu.guest_man)
                && let Some(label) = &mut w.label
            {
                if w.path.ends_with("/Text Free Comment") {
                    label.set_text(&man.comment);
                } else if w.path.ends_with("/Favorite/Value") {
                    label.set_text(&man.favorite);
                }
            }
            if menu.page == Page::Main
                && d == FreeMode
                && w.path.ends_with("/ExplanatoryText/Value")
                && let Some(label) = &mut w.label
            {
                label.set_text("回想のジャンルを選択してください。");
            }
            let confirm = w
                .path
                .strip_prefix(prefix)
                .is_some_and(|path| matches!(path, "OK" | "Ok" | "ButtonParent/OK"));
            if menu.page != Page::Main
                && menu.page != Page::GuestHeroine
                && confirm
                && menu.selected.is_none()
            {
                w.collider = None;
                w.tint[3] = w.tint[3].min(100);
            }
            if w.path.ends_with("/DetailBlock/Text")
                && let Some(label) = &mut w.label
            {
                label.set_text(
                    menu.selected
                        .and_then(|i| menu.rows.get(i))
                        .map_or("", |r| &r.detail),
                );
            }
        }
        if menu.page == Page::Main {
            return Ok(());
        }
        let horizontal = matches!(menu.page, Page::Stage | Page::GuestRooms);
        let capacity = if horizontal { 5 } else { 10 };
        for (index, row) in menu
            .rows
            .iter()
            .enumerate()
            .skip(menu.offset)
            .take(capacity)
        {
            let n = (index - menu.offset) as f32;
            let [dx, dy] = match menu.page {
                Page::Stage => [160. + n * 325. - 960., 755. - 540.],
                Page::GuestRooms => [310. + n * 300. - 960., 460. - 540.],
                // Level 60's Window Man List uses the scene item's
                // authored origin and a 50 px vertical UIGrid step.
                Page::GuestMan => [0., n * 50.],
                _ => [292. - 960., 244. + n * 56. - 540.],
            };
            for base in &menu.template {
                let mut w = base.clone();
                let name = base.path.rsplit('/').next().unwrap_or_default();
                if matches!(name, "SelectFrame" | "SelectCursor") && menu.selected != Some(index) {
                    continue;
                }
                if (menu.page == Page::GuestMan || matches!(name, "Name" | "Title" | "StageName" | "Label Name"))
                    && let Some(label) = &mut w.label
                {
                    label.set_text(if menu.page == Page::Stage && !row.enabled {
                        "??????"
                    } else {
                        &row.title
                    });
                }
                if menu.page == Page::Stage
                    && name == "Icon"
                    && let Some(asset) = &menu.stages[&row.key].asset
                {
                    w.asset = asset.clone();
                    w.uv = [0., 0., 1., 1.];
                }
                business::move_by(&mut w, dx, dy);
                w.depth += 200;
                let hit = w.collider.or_else(|| {
                    (w.path.split('/').count() == 1 || name == "temp item").then_some(w.rect)
                });
                w.collider = if row.enabled { hit } else { None };
                if menu.page == Page::GuestMan && menu.selected == Some(index) {
                    w.tint[3] = 255;
                }
                if !row.enabled {
                    if menu.page != Page::Stage {
                        w.tint[3] = w.tint[3].min(120);
                    }
                }
                w.path = format!("native-row/{index}/{}", w.path);
                self.widgets.push(w);
            }
        }
        Ok(())
    }

    pub(in crate::com3d2) fn show_guest_man(&mut self, id: i32, op: &mut Opener, now: u64) -> Result<()> {
        let profile = self.leisure.native_menu.guest_men.iter().find(|man| man.id == id)
            .with_context(|| format!("unavailable guest man {id}"))?;
        let preset = if profile.id == guest::PROTAGONIST {
            None
        } else {
            ensure!(!profile.preset.is_empty(), "missing guest NPC preset {}", profile.id);
            Some(profile.preset.as_str())
        };
        self.actors.display_guest_man(
            &self.script, &self.files, &mut self.assets, op, self.editor.player_model(), preset, now,
        )?;
        self.leisure.native_menu.guest_man = Some(id);
        Ok(())
    }

    pub(in crate::com3d2) fn close_guest_comment(&mut self, h: &mut dyn HostServices, c: ClientId) -> Result<()> {
        if let Some(comment) = self.leisure.native_menu.guest_comment.take() {
            comment.close(h, c)?;
        }
        Ok(())
    }

    pub(super) fn update_guest_comment(&mut self, h: &mut dyn HostServices, c: ClientId, input: InputFrame) -> Result<bool> {
        let Some(comment) = &mut self.leisure.native_menu.guest_comment else { return Ok(false) };
        if let Some((maid, text)) = comment.update(h, c, input, self.assets.guest_comment_limit)? {
            let mut vm = self.script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
            guest::set_comment(&mut vm.runtime.host_mut().state, maid, text)?;
            drop(vm);
            self.draw_native_menu(GuestClub)?;
        }
        let exit_key = input.buttons_pressed & (buttons::ENTER | buttons::ESCAPE) != 0;
        if exit_key || input.buttons_pressed & buttons::MOUSE_LEFT != 0 {
            self.close_guest_comment(h, c)?;
        }
        Ok(exit_key)
    }

    pub(in crate::com3d2) fn resume_recollection(
        &mut self,
        name: &str,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<()> {
        let returns = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?
            .runtime
            .host()
            .state
            .retail
            .recollection_returns
            .clone()
            .context("no saved recollection return labels")?;
        self.script.tag_backup = returns;
        self.enter(Scene::Leisure(FreeMode), op, h, c, now)?;
        let button = match name {
            "Select" => return Ok(()),
            "Story" => "ストーリー",
            "Everyday" => "日常",
            "Vip" => return self.open_vip(op, h, c),
            other => bail!("unknown recollection screen {other}"),
        };
        self.native_menu_click(FreeMode, button, op, h, c, now)
    }

    pub(super) fn native_menu_scroll(&mut self, d: Destination, input: InputFrame) -> Result<()> {
        if self.leisure.native_menu.page == Page::GuestReplay {
            return self.guest_replay_input(input);
        }
        if self.leisure.native_menu.page == Page::SubCast {
            return self.sub_cast_scroll(d, input);
        }
        if self.leisure.native_menu.page == Page::FreeSkill {
            return self.free_skill_scroll(d, input);
        }
        if self.leisure.native_menu.page == Page::Vip {
            return self.vip_update(input);
        }
        let menu = &mut self.leisure.native_menu;
        if menu.page != Page::Main
            && input.buttons_pressed & (buttons::WHEEL_UP | buttons::WHEEL_DOWN) != 0
        {
            menu.offset = if input.buttons_pressed & buttons::WHEEL_UP != 0 {
                menu.offset.saturating_sub(1)
            } else {
                (menu.offset + 1).min(menu.rows.len().saturating_sub(1))
            };
            self.draw_native_menu(d)?;
        }
        Ok(())
    }

    fn native_menu_template(&self, prefix: &str) -> Vec<Widget> {
        let mut rows = self
            .leisure
            .base
            .iter()
            .filter(|w| {
                w.path == prefix
                    || w.path
                        .strip_prefix(prefix)
                        .is_some_and(|tail| tail.starts_with('/'))
            })
            .cloned()
            .collect::<Vec<_>>();
        if let Some(widget) = rows.iter_mut().find(|w| w.path == prefix) {
            widget.collider = Some(widget.rect);
        }
        rows
    }

    fn launch_guest_scenario(
        &mut self,
        maid: usize,
        room: i32,
        man: i32,
        replay: Option<String>,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<()> {
        let label = if replay.is_some() {
            "label_free"
        } else {
            "label"
        };
        ensure!(
            self.script.tag_backup.contains_key(label),
            "missing guest script return attribute {label}"
        );
        {
            let mut vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let s = &mut vm.runtime.host_mut().state;
            let file = match replay.as_ref() {
                Some(file) => file.clone(),
                None => guest::optimal(s, maid, room, man)?.file,
            };
            s.active_maids.resize(s.active_maids.len().max(1), None);
            let previous = s.active_maids[0].replace(maid);
            let resolved = if file.contains('?') {
                crate::tjs::retail::replace_script_name(s, &file).transpose()
            } else {
                Ok(None)
            };
            s.active_maids[0] = previous;
            let file = resolved?.unwrap_or(file);
            let path = if file.to_ascii_lowercase().ends_with(".ks") {
                file.clone()
            } else {
                format!("{file}.ks")
            };
            if !self.files.exists(&path) {
                drop(vm);
                return self.leisure_prompt(
                    op,
                    h,
                    c,
                    now,
                    "シナリオファイルが見つかりません。",
                    "ok",
                    Response::Dismiss,
                );
            }
            if replay.is_some() {
                guest::use_man(s, man)?;
            }
            guest::select_room(s, room);
            s.active_maids[0] = Some(maid);
            if replay.is_some() {
                vm.eval(&format!(
                    "tf['傅き再生_ファイル名'] = {}",
                    serde_json::to_string(&file)?
                ))?;
            }
        }
        self.actors
            .activate_stock(&self.script, maid, 0, Some(true))?;
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
        )?;
        self.resume_at(label, op, h, c, now)
    }

    pub(super) fn native_menu_click(
        &mut self,
        d: Destination,
        path: &str,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<()> {
        use Page::*;
        if d == GuestClub && self.leisure.native_menu.page == GuestReplay {
            return self.guest_replay_click(path, op, h, c, now);
        }
        if self.leisure.native_menu.page == SubCast {
            return self.sub_cast_click(d, path, op, h, c, now);
        }
        if self.leisure.native_menu.page == FreeSkill {
            return self.free_skill_click(d, path, op, h, c, now);
        }
        if self.leisure.native_menu.page == Vip {
            return self.vip_click(path, op, h, c, now);
        }
        if let Some(index) = path
            .strip_prefix("native-row/")
            .and_then(|p| p.split('/').next())
        {
            let i = index.parse::<usize>()?;
            let menu = &self.leisure.native_menu;
            let page = menu.page;
            if let Some(row) = menu.rows.get(i).filter(|r| r.enabled).cloned() {
                if page == Stage {
                    let preview = &menu.stages[&row.key];
                    let orbit = preview.camera;
                    let backdrop = preview.backdrop.clone();
                    let file = crate::sound::ogg(&preview.music);
                    let changed = menu
                        .selected
                        .and_then(|i| menu.rows.get(i))
                        .is_none_or(|r| menu.stages[&r.key].backdrop != backdrop);
                    if changed {
                        let model = club::prefab(&mut self.assets.unity, op, &backdrop)?;
                        self.replace_leisure_model(&backdrop, model, h, c)?;
                    }
                    self.adv_camera = orbit;
                    let mut command = crate::bg::orbit_camera(orbit.0, orbit.2, orbit.1, 35.);
                    command["op"] = json!("set_camera");
                    h.submit_engine(c, 1, &serde_json::to_vec(&json!({"world":[command]}))?)?;
                    self.native_menu_music(d, &file, 1., h, c)?;
                    let mut vm = self
                        .script
                        .tjs
                        .lock()
                        .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                    let s = &mut vm.runtime.host_mut().state;
                    s.stage_name = row.key;
                    s.stage_prefab = backdrop;
                } else if page == GuestMan {
                    let id = row.key.parse()?;
                    self.show_guest_man(id, op, now)?;
                    let mut vm = self
                        .script
                        .tjs
                        .lock()
                        .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                    guest::use_man(&mut vm.runtime.host_mut().state, id)?;
                }
                self.leisure.native_menu.selected = Some(i);
            }
            return self.draw_native_menu(d);
        }
        let control = path.rsplit('/').next().unwrap_or_default();
        let (page, selected) = {
            let menu = &self.leisure.native_menu;
            (
                menu.page,
                menu.selected.and_then(|i| menu.rows.get(i)).cloned(),
            )
        };
        let mut next = page;
        let mut rows = Vec::new();
        if d == GuestClub && let Some(maid) = self.leisure.native_menu.selected_guest_maid()? {
            if page == GuestHeroine && path == guest_profile::COMMENT {
                let rect = self.widgets.iter().find(|w| w.path == path).and_then(|w| w.collider)
                    .context("guest comment hit box")?;
                let mut vm = self.script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
                let text = guest::overview(&mut vm.runtime.host_mut().state, maid)?.comment;
                drop(vm);
                self.close_guest_comment(h, c)?;
                self.leisure.native_menu.guest_comment = Some(guest_profile::Comment::open(maid, text, rect, h, c)?);
                return Ok(());
            }
            if page != GuestFree && path.contains("/Window Parameter Lock/") && matches!(control, "好感度" | "仕事回数") {
                let mut vm = self.script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
                guest::toggle_parameter_lock(&mut vm.runtime.host_mut().state, control == "好感度");
                drop(vm);
                return self.draw_native_menu(d);
            }
        }
        match (d, page, control) {
            (FreeMode, Main, "Cancel") => {
                self.script
                    .tjs
                    .lock()
                    .map_err(|_| anyhow::anyhow!("TJS lock"))?
                    .runtime
                    .host_mut()
                    .state
                    .retail
                    .recollection_returns = None;
                return self.resume_at("cancel_label", op, h, c, now);
            }
            (FreeMode, Main, "夜伽") => return self.resume_at("yotogi_label", op, h, c, now),
            (FreeMode, Main, "VIP" | "VIP_HEvent") => return self.open_vip(op, h, c),
            (FreeMode, Main, "ストーリー" | "日常") => {
                let m = &self.leisure.native_menu;
                (next, rows) = match control {
                    "ストーリー" => (Story, m.story.clone()),
                    _ => (Everyday, m.everyday.clone()),
                };
            }
            (FreeMode, Story | Everyday | Vip, "Cancel") => next = Main,
            (FreeMode, Story | Everyday | Vip, "OK") => {
                if let Some(row) = selected {
                    let key = match page {
                        Story => "scene_label_mainstory",
                        Everyday => "scene_label_everyday",
                        _ => "scene_label_vip",
                    };
                    {
                        let mut vm = self
                            .script
                            .tjs
                            .lock()
                            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                        let s = &mut vm.runtime.host_mut().state;
                        let file = if row.key.contains('?') {
                            crate::tjs::retail::replace_script_name(s, &row.key)
                                .transpose()?
                                .unwrap_or(row.key)
                        } else {
                            row.key
                        };
                        vm.eval(&format!(
                            "tf['シーン再生_ファイル名'] = {}",
                            serde_json::to_string(&file)?
                        ))?;
                    }
                    self.sound.stop(h, c, Kind::Bgm, 0.3)?;
                    return self.resume_at(key, op, h, c, now);
                }
            }
            (GuestClub, Main, "タイトルに戻る") => {
                return self.leisure_prompt(
                    op,
                    h,
                    c,
                    now,
                    "保存されていない情報は失われます。\nタイトルに戻りますか？",
                    "yesno",
                    Response::GuestTitle,
                );
            }
            (GuestClub, Main, "男選択") => {
                rows = self.leisure.native_menu.guest_men
                    .iter()
                    .map(|man| Row {
                        title: man.title.clone(),
                        detail: String::new(),
                        key: man.id.to_string(),
                        enabled: true,
                    })
                    .collect();
                self.leisure.native_menu.template = self.native_menu_template(
                    "UI Root/Window Select Man/Window Man List/ScrollView/Grid/temp item",
                );
                next = GuestMan;
            }
            (GuestClub, GuestMan, "OK" | "Cancel") => next = Main,
            (GuestClub, GuestHeroine, "OK" | "Cancel")
            | (GuestClub, GuestFree | GuestRooms, "Cancel") => next = Main,
            (GuestClub, Main, "クラブ") | (GuestClub, GuestRoomMaid, "Cancel") => {
                let mut vm = self
                    .script
                    .tjs
                    .lock()
                    .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                let s = &mut vm.runtime.host_mut().state;
                let man = guest::man(s);
                rows = guest_rooms(s, man)?;
                drop(vm);
                let message = if rows.is_empty() {
                    Some("利用可能な施設がありません。")
                } else if !rows.iter().any(|r| r.enabled) {
                    Some("プレイ可能なメイドがいません。")
                } else {
                    None
                };
                if let Some(message) = message {
                    return self.leisure_prompt(op, h, c, now, message, "ok", Response::Dismiss);
                }
                self.leisure.native_menu.template = self
                    .native_menu_template("UI Root/Window Select Club/ScrollView/Grid/temp item");
                next = GuestRooms;
            }
            (GuestClub, GuestRooms, "OK") => {
                if let Some(row) = selected {
                    let room = row.key.parse()?;
                    let mut vm = self
                        .script
                        .tjs
                        .lock()
                        .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                    let s = &mut vm.runtime.host_mut().state;
                    let man = guest::man(s);
                    let ids = guest::room_roster(s, room, man)?;
                    if !ids.is_empty() {
                        guest::select_room(s, room);
                        rows = guest_roster(s, ids);
                        self.leisure.native_menu.guest_room = Some((room, man));
                        drop(vm);
                        self.leisure.native_menu.template =
                            self.assets.scene(op, h, c, "RecollectionRow")?;
                        next = GuestRoomMaid;
                    }
                }
            }
            (GuestClub, Main, "ヒロイン" | "フリー") => {
                let vm = self
                    .script
                    .tjs
                    .lock()
                    .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                let s = &vm.runtime.host().state;
                rows = guest_roster(s, guest::eligible_maids(s));
                drop(vm);
                self.leisure.native_menu.template =
                    self.assets.scene(op, h, c, "RecollectionRow")?;
                next = if control == "フリー" {
                    GuestFree
                } else {
                    GuestHeroine
                };
            }
            (GuestClub, GuestRoomMaid, "OK") => {
                if let Some(row) = selected {
                    let (room, man) = self
                        .leisure
                        .native_menu
                        .guest_room
                        .context("missing guest room selection")?;
                    let id = row.key.parse()?;
                    return self.launch_guest_scenario(id, room, man, None, op, h, c, now);
                }
            }
            (GuestClub, GuestFree, "OK") => {
                if let Some(row) = selected {
                    return self.open_guest_replay(row.key.parse()?);
                }
            }
            (Yotogi | OldYotogi, Stage, "Ok") => {
                if selected.is_some() && self.script.tag_backup.contains_key("label") {
                    return self.resume_at("label", op, h, c, now);
                }
            }
            (_, _, "Up" | "Left" | "Down" | "Right") if page != Main => {
                let menu = &mut self.leisure.native_menu;
                menu.offset = if matches!(control, "Up" | "Left") {
                    menu.offset.saturating_sub(1)
                } else {
                    (menu.offset + 1).min(menu.rows.len().saturating_sub(1))
                };
                return self.draw_native_menu(d);
            }
            (_, _, "Title" | "Back") => {}
            (GuestClub, Main, "セーブ" | "ロード") => {
                return self.open_guest_slots(
                    if control == "セーブ" { super::super::slots::Mode::GuestSave }
                    else { super::super::slots::Mode::GuestLoad },
                    op, h, c, now,
                );
            }
            _ => {}
        }
        if next != page {
            let menu = &mut self.leisure.native_menu;
            menu.page = next;
            menu.selected = if next == GuestMan {
                menu.rows.iter().position(|r| r.key.parse::<i32>().ok() == menu.guest_man)
            } else if matches!(next, GuestFree | GuestRooms | GuestRoomMaid | GuestHeroine) {
                None
            } else {
                rows.iter().position(|row| row.enabled)
            };
            menu.rows = rows;
            menu.offset = 0;
            self.draw_native_menu(d)?;
        }
        Ok(())
    }
}
