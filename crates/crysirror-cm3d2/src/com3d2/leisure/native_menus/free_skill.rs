// SPDX-License-Identifier: MPL-2.0
//! Level 14's embedded selector and level 70's instantiated legacy panel.
use super::*;
use crate::tjs::retail::free_skill as native;
use crysirror_unity::reader::Reader;

pub(in crate::com3d2) const PREFIX: &str = "UI Root/FreeSkillSelectPanel/";
const PANEL: &str = "UI Root/FreeSkillSelectPanel/FreeSkillSelectParent/";
const ACTIONS: &str = "UI Root/FreeSkillSelectPanel/CommonPanel/UnderButtonGroup/";
const VIEWERS: [&str; 5] = [
    "TypeCategoryViewer",
    "CategoryViewer",
    "SkillSelectViewer",
    "LevelViewer",
    "StageSelectViewer",
];

pub(in crate::com3d2) fn is_free_skill(
    destination: Destination,
    tag: &std::collections::BTreeMap<String, String>,
) -> bool {
    matches!(destination, Yotogi | OldYotogi)
        && match tag.get("start") {
            Some(screen) => screen == "FreeSkillSelect",
            None => tag.contains_key("free_start"),
        }
}

#[derive(Clone, Copy, Default)]
struct Grid {
    origin: [f32; 2],
    clip: [f32; 4],
    step: f32,
    pivot_y: f32,
}

#[derive(Clone, Default)]
pub(in crate::com3d2) struct Layout([Grid; 5]);

impl Layout {
    pub(in crate::com3d2) fn read(
        &mut self,
        class: &str,
        path: &str,
        mut offset: [f32; 3],
        r: &mut Reader<'_>,
        old: bool,
    ) -> Result<()> {
        let Some((_, path)) = path.split_once("FreeSkillSelect/") else {
            return Ok(());
        };
        let Some((viewer, control)) = path.split_once('/') else {
            return Ok(());
        };
        let Some(i) = VIEWERS.iter().position(|&name| name == viewer) else {
            return Ok(());
        };
        // Both scenes attach the selector to FreeSkillSelectParent at y=-50.
        if old {
            offset[1] -= 50.;
        }
        let grid = &mut self.0[i];
        match (class, control) {
            ("UIGrid", "Content/GridParent") => {
                r.seek(32)?;
                ensure!(r.i32()? == 1, "unexpected free skill grid arrangement");
                r.seek(40)?;
                let pivot = r.i32()?;
                ensure!((0..=8).contains(&pivot), "invalid free skill grid pivot");
                grid.pivot_y = (pivot / 3) as f32 * 0.5;
                r.seek(52)?;
                grid.step = r.f32()?;
                grid.origin = [offset[0], -offset[1]];
            }
            ("UIPanel", "Content") => {
                r.seek(172)?;
                let [x, y, w, h] = r.f32s::<4>()?;
                r.seek(204)?;
                let [dx, dy] = r.f32s::<2>()?;
                grid.clip = [
                    960. + offset[0] + x + dx - w / 2.,
                    540. - offset[1] - y - dy - h / 2.,
                    w,
                    h,
                ];
            }
            _ => {}
        }
        Ok(())
    }

    pub(in crate::com3d2) fn clip(&self, path: &str) -> Option<[f32; 4]> {
        let column = path
            .strip_prefix("free-skill/")?
            .split('/')
            .next()?
            .parse::<usize>()
            .ok()?;
        self.0.get(column).map(|grid| grid.clip)
    }
}

struct Cell<'a> {
    id: usize,
    title: &'a str,
    enabled: bool,
    selected: bool,
}

pub(super) struct Picker {
    data: native::Catalogue,
    templates: [Vec<Widget>; 3],
    thumbnails: Vec<Option<String>>,
    offsets: [usize; 5],
    kind: usize,
    category: usize,
    skill: Option<usize>,
    stage: Option<usize>,
    preferred_stage: Option<usize>,
    level: u8,
    old: bool,
}

impl Picker {
    fn types(&self) -> &[&str] {
        if self.old {
            &native::LEGACY_TYPES
        } else {
            &native::CURRENT_TYPES
        }
    }

    fn categories(&self, kind: usize) -> impl Iterator<Item = Cell<'_>> {
        native::CATEGORIES
            .iter()
            .take(if self.old { 9 } else { 8 })
            .enumerate()
            .filter_map(move |(id, title)| {
                let available = self
                    .data
                    .skills
                    .iter()
                    .any(|skill| skill.kind == kind && skill.category == id);
                if kind != 0 && !available || self.data.locked && matches!(id, 4 | 5) {
                    return None;
                }
                Some(Cell {
                    id,
                    title,
                    enabled: available,
                    selected: self.category == id,
                })
            })
    }

    fn stage_enabled(&self, id: usize) -> bool {
        self.skill.is_some_and(|skill| {
            self.data.skills[skill]
                .stages
                .contains(&self.data.stages[id].key)
        })
    }

    fn cells(&self, column: usize) -> Vec<Cell<'_>> {
        match column {
            0 => self
                .types()
                .iter()
                .enumerate()
                .filter_map(|(id, title)| {
                    let enabled = self.categories(id).any(|c| c.enabled);
                    (self.old || id == 0 || enabled).then_some(Cell {
                        id,
                        title,
                        enabled,
                        selected: id == self.kind,
                    })
                })
                .collect(),
            1 => self.categories(self.kind).collect(),
            2 => self
                .data
                .skills
                .iter()
                .enumerate()
                .filter(|(_, skill)| {
                    skill.kind == self.kind
                        && skill.category == self.category
                        && (!self.data.locked || !matches!(skill.category, 4 | 5))
                })
                .map(|(id, skill)| Cell {
                    id,
                    title: &skill.name,
                    enabled: true,
                    selected: self.skill == Some(id),
                })
                .collect(),
            3 => ["Lv.1", "Lv.2", "Lv.3"]
                .into_iter()
                .enumerate()
                .map(|(id, title)| Cell {
                    id,
                    title,
                    enabled: true,
                    selected: id + 1 == self.level as usize,
                })
                .collect(),
            4 => self
                .data
                .stages
                .iter()
                .enumerate()
                .map(|(id, stage)| Cell {
                    id,
                    title: &stage.name,
                    enabled: self.stage_enabled(id),
                    selected: self.stage == Some(id),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn choose(&mut self, column: usize, id: usize) {
        if !self
            .cells(column)
            .iter()
            .any(|cell| cell.enabled && cell.id == id)
        {
            return;
        }
        match column {
            0 => {
                self.kind = id;
                self.offsets[1] = 0;
                let first = self
                    .categories(id)
                    .find(|cell| cell.enabled)
                    .map(|cell| cell.id);
                if let Some(category) = first {
                    self.choose(1, category);
                }
            }
            1 => {
                self.category = id;
                self.offsets[2] = 0;
                let first = self.cells(2).first().map(|cell| cell.id);
                if let Some(skill) = first {
                    self.choose(2, skill);
                }
            }
            2 => {
                self.skill = Some(id);
                self.stage = self
                    .preferred_stage
                    .filter(|&stage| self.stage_enabled(stage))
                    .or_else(|| {
                        (0..self.data.stages.len()).find(|&stage| self.stage_enabled(stage))
                    });
            }
            3 => self.level = (id + 1) as u8,
            4 => {
                self.stage = Some(id);
                self.preferred_stage = self.stage;
            }
            _ => {}
        }
    }

    fn current(&self) -> Option<(native::Choice, &str)> {
        let stage = &self.data.stages[self.stage?];
        Some((
            native::Choice {
                skill: self.data.skills[self.skill?].id,
                level: self.level,
                stage: stage.key.clone(),
                day: stage.day,
            },
            &stage.backdrop,
        ))
    }

    fn select_initial(&mut self) {
        if let Some(previous) = &self.data.previous
            && let Some(skill) = self.data.skills.iter().position(|s| s.id == previous.skill)
            && (!self.data.locked || !matches!(self.data.skills[skill].category, 4 | 5))
        {
            self.preferred_stage = self
                .data
                .stages
                .iter()
                .position(|stage| stage.key == previous.stage && stage.day == previous.day);
            self.level = previous.level;
            self.kind = self.data.skills[skill].kind;
            self.category = self.data.skills[skill].category;
            self.choose(2, skill);
        } else if let Some(first) = self
            .cells(0)
            .into_iter()
            .find(|cell| cell.enabled)
            .map(|cell| cell.id)
        {
            self.choose(0, first);
        }
    }

    fn capacity(grid: Grid) -> usize {
        (grid.clip[3] / grid.step).floor().max(1.) as usize
    }

    fn reveal(&mut self, layout: &Layout) {
        for (column, grid) in layout.0.iter().enumerate() {
            let cells = self.cells(column);
            let selected = cells.iter().position(|c| c.selected);
            let capacity = Self::capacity(*grid);
            if let Some(i) = selected {
                self.offsets[column] = self.offsets[column]
                    .min(i)
                    .max((i + 1).saturating_sub(capacity));
            }
        }
    }
}

impl Game {
    pub(super) fn open_free_skill(
        &mut self,
        d: Destination,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<String> {
        let old = d == OldYotogi;
        if old {
            let mut old_panel = self.assets.scene(op, host, c, "FreeSkillOldPanel")?;
            for widget in &mut old_panel {
                business::move_by(widget, 0., 50.);
                widget.path.insert_str(0, PANEL);
            }
            self.leisure.base.extend(old_panel);
        }
        ensure!(
            self.assets
                .free_skill_layout
                .0
                .iter()
                .all(|g| g.step.is_finite() && g.step > 0. && g.clip[3] > 0.),
            "free skill selector is missing an authored grid"
        );
        let templates = [
            self.assets.scene(op, host, c, "FreeSkillCategory")?,
            self.assets.scene(op, host, c, "FreeSkillRow")?,
            self.assets.scene(
                op,
                host,
                c,
                if old {
                    "StageSelectionOldRow"
                } else {
                    "StageSelectionRow"
                },
            )?,
        ];
        let mut vm = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let s = &mut vm.runtime.host_mut().state;
        let data = native::catalogue(s, old)?;
        let lead = s
            .active_maids
            .first()
            .copied()
            .flatten()
            .filter(|id| s.retail.stock_models.contains_key(id));
        let has_world = !s.bg_name.is_empty()
            && self
                .assets
                .unity
                .resource_of(
                    op,
                    &format!("bg/{}", s.bg_name),
                    crysirror_unity::class_id::GAME_OBJECT,
                )?
                .is_some();
        let backdrop = if !has_world {
            data.stages
                .first()
                .map_or_else(|| "Theater".to_owned(), |stage| stage.backdrop.clone())
        } else {
            s.bg_name.clone()
        };
        let mut thumbnails = Vec::with_capacity(data.stages.len());
        for stage in &data.stages {
            let filename = if !old && stage.thumbnail.contains('?') {
                crate::tjs::retail::replace_script_name(s, &stage.thumbnail)
                    .transpose()?
                    .unwrap_or_else(|| stage.thumbnail.clone())
            } else {
                stage.thumbnail.clone()
            };
            let files = if old {
                s.retail.linked_files()
            } else {
                &self.files
            };
            let stem = filename
                .rsplit_once('.')
                .map_or(filename.as_str(), |(stem, _)| stem);
            let thumbnail = [stem, "cm3d2_stageselectpreview_dammy"]
                .into_iter()
                .find(|name| files.exists(&format!("{name}.tex")))
                .map(|name| self.assets.archive_icon(op, host, c, files, name, old))
                .transpose()?;
            thumbnails.push(thumbnail);
        }
        let preferred_stage = old
            .then(|| data.stages.iter().position(|stage| stage.key == "サロン"))
            .flatten();
        let mut picker = Picker {
            data,
            templates,
            thumbnails,
            offsets: [0; 5],
            kind: 0,
            category: 0,
            skill: None,
            stage: None,
            preferred_stage,
            level: 1,
            old,
        };
        picker.select_initial();
        picker.reveal(&self.assets.free_skill_layout);
        if let Some((choice, backdrop)) = picker.current() {
            native::apply(s, choice, backdrop)?;
        }
        drop(vm);
        if let Some(id) = lead {
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
        for slot in 1..18 {
            if let Some(actor) = self.actors.actor(false, slot) {
                actor.visible = false;
            }
        }
        if let Some(actor) = self.actors.actor(true, 0) {
            actor.visible = false;
        }
        self.leisure.native_menu.page = Page::FreeSkill;
        self.leisure.native_menu.free_skill = Some(picker);
        self.draw_native_menu(d)?;
        Ok(backdrop)
    }

    pub(super) fn draw_free_skill(&mut self) -> Result<()> {
        let picker = self
            .leisure
            .native_menu
            .free_skill
            .as_ref()
            .context("free skill selector is closed")?;
        self.widgets.retain_mut(|widget| {
            let Some(control) = widget.path.strip_prefix(ACTIONS) else {
                return !widget.path.contains("/Arrow") || !widget.path.starts_with(PANEL);
            };
            let enabled = match control {
                "Next" => picker.current().is_some() && has_label(&self.script, "label"),
                "Cancel" => has_label(&self.script, "cancel_label"),
                _ => return false,
            };
            if !enabled {
                widget.collider = None;
                widget.tint[3] = widget.tint[3].min(100);
            }
            true
        });
        for (column, grid) in self.assets.free_skill_layout.0.iter().enumerate() {
            let offset = picker.offsets[column];
            let cells = picker.cells(column);
            let capacity = Picker::capacity(*grid);
            let shift =
                cells.len().min(capacity).saturating_sub(1) as f32 * grid.step * grid.pivot_y;
            for (visible, cell) in cells.into_iter().skip(offset).take(capacity).enumerate() {
                let template = &picker.templates[match column {
                    2 => 1,
                    4 => 2,
                    _ => 0,
                }];
                let dy = visible as f32 * grid.step - shift;
                for base in template {
                    let mut widget = base.clone();
                    let name = base.path.rsplit('/').next().unwrap_or_default();
                    if matches!(name, "SelectFrame" | "SelectCursor") && !cell.selected {
                        continue;
                    }
                    if let Some(label) = &mut widget.label
                        && matches!(name, "Label" | "title" | "StageName" | "Name")
                    {
                        label.set_text(cell.title);
                    }
                    if column == 4
                        && name == "Icon"
                        && let Some(asset) = &picker.thumbnails[cell.id]
                    {
                        widget.asset.clone_from(asset);
                        widget.uv = [0., 0., 1., 1.];
                    }
                    business::move_by(&mut widget, grid.origin[0], grid.origin[1] + dy);
                    widget.depth += 50;
                    if !cell.enabled {
                        widget.collider = None;
                        widget.tint[3] = widget.tint[3].min(120);
                    } else if matches!(name, "Button" | "BG") {
                        widget.collider = widget.collider.or(Some(widget.rect));
                    }
                    widget.collider = widget
                        .collider
                        .and_then(|rect| crate::ngui::clip_collider(rect, grid.clip));
                    widget.path = format!("free-skill/{column}/{}/{}", cell.id, widget.path);
                    self.widgets.push(widget);
                }
                if cell.selected {
                    let marker = format!("{PANEL}FreeSkillSelect/{}/Arrow", VIEWERS[column]);
                    if let Some(mut arrow) =
                        self.leisure.base.iter().find(|w| w.path == marker).cloned()
                    {
                        let y = grid.origin[1] + 522. + dy - arrow.rect[3] / 2.;
                        let delta = y - arrow.rect[1];
                        business::move_by(&mut arrow, 0., delta);
                        arrow.depth += 51;
                        arrow.collider = None;
                        self.widgets.push(arrow);
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn free_skill_scroll(&mut self, d: Destination, input: InputFrame) -> Result<()> {
        if input.buttons_pressed & (buttons::WHEEL_UP | buttons::WHEEL_DOWN) == 0 {
            return Ok(());
        }
        let Some((column, grid)) = self
            .assets
            .free_skill_layout
            .0
            .iter()
            .enumerate()
            .find(|(_, grid)| business::contains(grid.clip, &input))
        else {
            return Ok(());
        };
        let picker = self
            .leisure
            .native_menu
            .free_skill
            .as_mut()
            .context("free skill selector is closed")?;
        let limit = picker
            .cells(column)
            .len()
            .saturating_sub(Picker::capacity(*grid));
        let offset = &mut picker.offsets[column];
        *offset = if input.buttons_pressed & buttons::WHEEL_UP != 0 {
            offset.saturating_sub(1)
        } else {
            (*offset + 1).min(limit)
        };
        self.draw_native_menu(d)
    }

    pub(super) fn free_skill_click(
        &mut self,
        d: Destination,
        path: &str,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<()> {
        let picker = self
            .leisure
            .native_menu
            .free_skill
            .as_mut()
            .context("free skill selector is closed")?;
        if let Some(path) = path.strip_prefix("free-skill/") {
            let mut segments = path.split('/');
            let column = segments
                .next()
                .context("free skill column")?
                .parse::<usize>()?;
            let id = segments
                .next()
                .context("free skill row")?
                .parse::<usize>()?;
            picker.choose(column, id);
            picker.reveal(&self.assets.free_skill_layout);
            if let Some((choice, backdrop)) = picker.current() {
                let mut vm = self
                    .script
                    .tjs
                    .lock()
                    .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                native::apply(&mut vm.runtime.host_mut().state, choice, backdrop)?;
            }
            return self.draw_native_menu(d);
        }
        let key = match path.strip_prefix(ACTIONS) {
            Some("Next") if picker.current().is_some() => "label",
            Some("Cancel") => "cancel_label",
            _ => return Ok(()),
        };
        if has_label(&self.script, key) {
            self.resume_at(key, op, h, c, now)?;
        }
        Ok(())
    }
}

fn has_label(script: &crate::kag::AdvKag, key: &str) -> bool {
    script
        .tag_backup
        .get(key)
        .is_some_and(|label| !label.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::com3d2::installed_menu::Menu;

    #[test]
    fn free_skill_profiles_keep_level_and_reject_disabled_stage() {
        for (old, second_kind) in [(false, 5), (true, 2)] {
            let stage = |key: &str, day| native::Stage {
                key: key.into(),
                name: key.into(),
                backdrop: format!("{key}-world"),
                thumbnail: String::new(),
                day,
            };
            let skill = |id, kind, category, stages: &[&str]| native::Skill {
                id,
                kind,
                category,
                name: format!("Skill {id}"),
                stages: stages.iter().map(|stage| (*stage).to_owned()).collect(),
            };
            let mut picker = Picker {
                data: native::Catalogue {
                    skills: vec![
                        skill(1, 0, 0, &["hall"]),
                        skill(2, second_kind, 1, &["room"]),
                        skill(3, 0, 4, &["hall"]),
                    ],
                    stages: vec![
                        stage("hall", true),
                        stage("hall", false),
                        stage("room", false),
                    ],
                    locked: true,
                    previous: None,
                },
                templates: Default::default(),
                thumbnails: Vec::new(),
                offsets: [0; 5],
                kind: 0,
                category: 0,
                skill: None,
                stage: None,
                preferred_stage: None,
                level: 1,
                old,
            };
            picker.select_initial();
            assert_eq!(picker.current().unwrap().0.skill, 1);
            assert!(!picker.cells(1).iter().any(|cell| matches!(cell.id, 4 | 5)));
            picker.choose(3, 2);
            picker.choose(4, 1);
            assert!(!picker.current().unwrap().0.day);
            picker.choose(0, second_kind);
            picker.choose(4, 1);
            let (choice, backdrop) = picker.current().unwrap();
            assert_eq!((choice.skill, choice.level, backdrop), (2, 3, "room-world"));
            picker.choose(0, 0);
            assert_eq!(picker.current().unwrap().0.stage, "hall");
            assert!(!picker.current().unwrap().0.day);
        }
    }

    #[test]
    #[ignore = "uses the installed standalone 1.0.4 selector and original scripts"]
    fn installed_free_skill_callbacks_and_layouts() -> Result<()> {
        let mut m = Menu::with_motion(true)?;
        let selector = Scene::Leisure(Yotogi);
        m.game
            .enter(Scene::Script, &mut m.op, &mut m.rt.host, m.client, 0)?;
        let tag = [
            ("name".into(), "SceneYotogi".into()),
            ("start".into(), "Null".into()),
            ("free_start".into(), String::new()),
        ]
        .into_iter()
        .collect();
        let mut vm = m.game.script.tjs.lock().unwrap();
        let state = &mut vm.runtime.host_mut().state;
        state.active_maids[0] = state.stock.first().copied();
        state.maid_mut(0).context("test character")?.cur_excite = 14;
        state.retail.scene_loaded("SceneYotogi", &tag);
        drop(vm);
        m.game.script.tag_backup = tag;
        m.game.script.load(
            &m.game.files,
            m.op.storage.as_ref(),
            "freemodemain.ks",
            "*free_yotogi_skill_select",
        )?;
        m.game.script.enabled = true;
        m.game.script.exec();
        m.wait(selector)?;
        let hit = |m: &Menu, path: &str, exact| {
            m.game
                .widgets
                .iter()
                .rev()
                .find(|widget| {
                    (if exact {
                        widget.path == path
                    } else {
                        widget.path.starts_with(path)
                    }) && widget.collider.is_some()
                })
                .and_then(|widget| widget.collider)
                .with_context(|| format!("missing free skill control {path}"))
        };
        assert!(
            m.game
                .widgets
                .iter()
                .any(|w| w.path.starts_with("free-skill/2/")
                    && w.label.as_ref().is_some_and(|label| !label.text.is_empty()))
        );
        let choice = |m: &Menu| {
            native::choice(&m.game.script.tjs.lock().unwrap().runtime.host().state).cloned()
        };
        assert_eq!(choice(&m).context("default selected free skill")?.level, 1);
        assert_eq!(
            m.game
                .script
                .tjs
                .lock()
                .unwrap()
                .runtime
                .host()
                .state
                .maid(0)
                .unwrap()
                .cur_excite,
            0
        );
        m.click(hit(&m, "free-skill/3/2/", false)?)?;
        assert_eq!(choice(&m).context("changed free skill")?.level, 3);
        m.click(hit(&m, &format!("{ACTIONS}Cancel"), true)?)?;
        assert_eq!(m.game.scene, Scene::Script);
        assert_eq!(
            m.game.script.position().split_whitespace().next(),
            Some("freemodemain.ks:9")
        );
        m.game.script.jump_label(
            &m.game.files,
            m.op.storage.as_ref(),
            "*free_yotogi_skill_select",
        )?;
        m.game.script.exec();
        m.wait(selector)?;
        assert_eq!(choice(&m).context("retained free skill")?.level, 3);
        m.click(hit(&m, &format!("{ACTIONS}Next"), true)?)?;
        assert_eq!(m.game.scene, Scene::Script);
        assert_eq!(
            m.game.script.position().split_whitespace().next(),
            Some("freemodemain.ks:46")
        );

        let modern = m.game.assets.free_skill_layout.clone();
        let legacy = m
            .game
            .assets
            .scene(&mut m.op, &mut m.rt.host, m.client, "SceneYotogiOld")?;
        assert!(
            legacy
                .iter()
                .any(|w| w.path == format!("{ACTIONS}Next") && w.collider.is_some())
        );
        let legacy =
            m.game
                .assets
                .scene(&mut m.op, &mut m.rt.host, m.client, "FreeSkillOldPanel")?;
        assert!(legacy.iter().any(|w| w.path.ends_with("/LevelViewer/Base")));
        assert_eq!(m.game.assets.free_skill_layout.0[2].step, modern.0[2].step);
        assert!(
            m.game
                .assets
                .free_skill_layout
                .0
                .iter()
                .all(|grid| grid.step > 0. && grid.clip[3] > 0.)
        );
        Ok(())
    }
}
