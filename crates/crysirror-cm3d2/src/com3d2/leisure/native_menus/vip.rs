// SPDX-License-Identifier: MPL-2.0
//! Level 24 ScheduleTaskViewer and DescScheduleYotogi.
use super::*;
use crate::tjs::retail::recollection::{self, Vip};

const MENU: &str = "UI Root/MenuVip/";
const VIEWER: &str = "UI Root/MenuVip/TaskViewer/";
const DESC: &str = "UI Root/MenuVip/TaskViewer/DescScheduleYotogi/";
const LISTS: [&str; 2] = ["UnitViewer/Content", "TaskViewer/Contents"];

#[derive(Clone, Default)]
pub(in crate::com3d2) struct Layout {
    grids: [shop::Grid; 2],
    y: [f32; 2],
    condition: [f32; 2],
    padding_y: f32,
}

#[test]
#[ignore = "installed standalone VIP catalogue and Unity prefabs"]
fn installed_vip_description_and_cancel() -> Result<()> {
    let mut m = crate::com3d2::installed_menu::Menu::with_motion(true)?;
    {
        let mut vm = m.game.script.tjs.lock().unwrap();
        let s = &mut vm.runtime.host_mut().state;
        s.active_maids[0] = s.stock.first().copied();
        let list = s.retail.table("recollection_vip2.nei")?;
        for row in (1..list.rows).filter(|&row| list.exists(0, row)) {
            let id = list.int(1, row);
            s.player.night_works_state.insert(
                id,
                crate::params::NightWorkState {
                    work_id: id,
                    finish: true,
                    ..Default::default()
                },
            );
        }
    }
    m.game.enter(
        Scene::Leisure(FreeMode),
        &mut m.op,
        &mut m.rt.host,
        m.client,
        0,
    )?;
    m.game.open_vip(&mut m.op, &mut m.rt.host, m.client)?;
    let screen = m.game.leisure.native_menu.vip.as_ref().unwrap();
    let selected = &screen.entries[screen.selected.context("first playable VIP")?];
    let message = m
        .game
        .widgets
        .iter()
        .find(|w| w.path == format!("{DESC}Description/Message"))
        .and_then(|w| w.label.as_ref())
        .context("VIP description label")?;
    assert_eq!(message.text, selected.replay.description);
    assert!(
        m.game
            .widgets
            .iter()
            .any(|w| w.path.starts_with("vip-work/") && w.collider.is_some())
    );
    let cancel = m
        .game
        .widgets
        .iter()
        .find(|w| w.path == format!("{MENU}Cancel"))
        .and_then(|w| w.collider)
        .context("VIP Cancel")?;
    m.click(cancel)?;
    assert_eq!(m.game.leisure.native_menu.page, Page::Main);
    Ok(())
}

impl Layout {
    pub(in crate::com3d2) fn wanted(path: &str) -> bool {
        path.strip_prefix(VIEWER).is_some_and(|p| {
            p.starts_with("DescScheduleYotogi/")
                || LISTS.iter().any(|prefix| p.starts_with(prefix))
                || p == "UnitViewer/Arrow"
                || p.starts_with("TaskViewer/Scroll Bar/")
        })
    }

    pub(in crate::com3d2) fn read(
        &mut self,
        class: &str,
        path: &str,
        offset: [f32; 3],
        data: &[u8],
    ) -> Result<()> {
        let Some(path) = path.strip_prefix(VIEWER) else {
            return Ok(());
        };
        if let Some(column) = LISTS.iter().position(|prefix| path.starts_with(prefix)) {
            self.grids[column].read(class, path, offset, data)?;
            if class == "UIGrid" {
                self.y[column] = -offset[1];
            }
        }
        if class == "UITable" && path == "DescScheduleYotogi/Condition/ConditionParent" {
            self.condition = [offset[0], -offset[1]];
            let mut r = crysirror_unity::reader::Reader::new(data);
            r.seek(64)?;
            self.padding_y = r.f32()?;
        }
        Ok(())
    }

    pub(in crate::com3d2) fn clip(&self, path: &str) -> Option<[f32; 4]> {
        if path.starts_with("vip-category/") {
            Some(self.grids[0].clip)
        } else if path.starts_with("vip-work/") {
            Some(self.grids[1].clip)
        } else {
            None
        }
    }
}

pub(super) struct Screen {
    entries: Vec<Vip>,
    categories: Vec<(i32, String)>,
    assets: Vec<Option<String>>,
    category: Option<i32>,
    selected: Option<usize>,
    hover: Option<usize>,
    offset: [usize; 2],
    templates: [Vec<Widget>; 3],
    hover_regions: Vec<([f32; 4], usize)>,
}

fn capacity(grid: shop::Grid) -> usize {
    (grid.clip[3] / grid.step).floor().max(1.) as usize
}

impl Game {
    pub(super) fn open_vip(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
    ) -> Result<()> {
        let templates = [
            self.assets.scene(op, host, c, "TrainingCategory")?,
            self.assets.scene(op, host, c, "VipTaskRow")?,
            self.assets.scene(op, host, c, "VipCondition")?,
        ];
        ensure!(
            self.assets
                .vip_layout
                .grids
                .iter()
                .all(|grid| grid.step > 0. && grid.clip[3] > 0.),
            "VIP scene grid layout"
        );
        let mut vm = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let (labels, entries) = recollection::vip(&mut vm.runtime.host_mut().state)?;
        let assets = entries
            .iter()
            .map(|entry| {
                let name = entry.icon.trim_end_matches(".tex");
                self.files
                    .exists(&format!("{name}.tex"))
                    .then(|| {
                        self.assets
                            .archive_icon(op, host, c, &self.files, name, false)
                    })
                    .transpose()
            })
            .collect::<Result<Vec<_>>>()?;
        drop(vm);
        let categories: Vec<_> = labels
            .into_iter()
            .filter(|(category, _)| entries.iter().any(|entry| entry.category == *category))
            .collect();
        let selected = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.replay.enabled)
            .min_by_key(|(_, entry)| (entry.category, entry.work))
            .map(|(i, _)| i);
        let category = selected
            .map(|i| entries[i].category)
            .or_else(|| categories.first().map(|v| v.0));
        self.leisure.native_menu.vip = Some(Screen {
            entries,
            categories,
            assets,
            category,
            selected,
            hover: None,
            offset: [0; 2],
            templates,
            hover_regions: Vec::new(),
        });
        self.leisure.native_menu.page = Page::Vip;
        self.draw_native_menu(FreeMode)
    }

    pub(super) fn draw_vip(&mut self) -> Result<()> {
        let screen = self
            .leisure
            .native_menu
            .vip
            .as_mut()
            .context("VIP viewer not open")?;
        let layout = &self.assets.vip_layout;
        let detail = screen.hover.or(screen.selected).map(|i| &screen.entries[i]);
        self.widgets.retain_mut(|widget| {
            if widget.path.contains("/FreeModeItemList/") {
                return false;
            }
            if widget.path == format!("{MENU}OK") && screen.selected.is_none() {
                widget.collider = None;
                widget.tint[3] = widget.tint[3].min(110);
            }
            if let Some(path) = widget.path.strip_prefix(DESC) {
                let Some(entry) = detail else { return false };
                if path.starts_with("MaidParameter/") {
                    return false;
                }
                if path.starts_with("Condition/") && entry.conditions.is_empty() {
                    return false;
                }
                if path == "Description/Message"
                    && let Some(label) = &mut widget.label
                {
                    label.set_text(&entry.replay.description);
                }
                if path == "BG" {
                    widget.rect[3] = 260.
                        + if entry.conditions.is_empty() {
                            0.
                        } else {
                            55. + entry.conditions.len() as f32 * (28. + layout.padding_y * 2.)
                        };
                }
            }
            true
        });
        if let Some(entry) = detail {
            for (i, text) in entry.conditions.iter().enumerate() {
                for template in &screen.templates[2] {
                    let mut widget = template.clone();
                    if let Some(label) = &mut widget.label {
                        if widget.path.ends_with("/Message") {
                            label.set_text(text);
                        } else if widget.path.ends_with("/No") {
                            label.set_text(&(i + 1).to_string());
                        }
                    }
                    business::move_by(
                        &mut widget,
                        layout.condition[0],
                        layout.condition[1] + i as f32 * (28. + layout.padding_y * 2.),
                    );
                    widget.path = format!("vip-condition/{i}/{}", widget.path);
                    self.widgets.push(widget);
                }
            }
        }
        screen.hover_regions.clear();
        for (column, grid) in layout.grids.iter().copied().enumerate() {
            let rows = if column == 0 {
                screen
                    .categories
                    .iter()
                    .enumerate()
                    .map(|(i, (id, label))| (i, label.as_str(), true, Some(*id) == screen.category))
                    .collect::<Vec<_>>()
            } else {
                screen
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| Some(row.category) == screen.category)
                    .map(|(i, row)| {
                        (
                            i,
                            row.replay.title.as_str(),
                            row.replay.enabled,
                            Some(i) == screen.selected,
                        )
                    })
                    .collect()
            };
            let prefix = if column == 0 {
                "vip-category"
            } else {
                "vip-work"
            };
            for (visible, (i, label, enabled, selected)) in rows
                .into_iter()
                .skip(screen.offset[column])
                .take(capacity(grid))
                .enumerate()
            {
                let y = layout.y[column] + visible as f32 * grid.step;
                for template in &screen.templates[column] {
                    let mut widget = template.clone();
                    let name = template.path.rsplit('/').next().unwrap_or_default();
                    if !selected && name == "SelectCursor" {
                        continue;
                    }
                    if let Some(text) = &mut widget.label {
                        text.set_text(label);
                    }
                    if column == 1 && name == "Main" {
                        if let Some(icon) = &screen.assets[i] {
                            widget.asset.clone_from(icon);
                        } else {
                            widget.tint[3] = 0;
                        }
                    }
                    business::move_by(&mut widget, grid.x, y);
                    if column == 1
                        && name == "Main"
                        && let Some(rect) = crate::ngui::clip_collider(widget.rect, grid.clip)
                    {
                        screen.hover_regions.push((rect, i));
                    }
                    if !enabled {
                        widget.collider = None;
                        widget.tint[3] = widget.tint[3].min(140);
                    }
                    widget.collider = widget
                        .collider
                        .and_then(|r| crate::ngui::clip_collider(r, grid.clip));
                    widget.depth += 100;
                    widget.path = format!("{prefix}/{i}/{}", widget.path);
                    self.widgets.push(widget);
                }
            }
        }
        Ok(())
    }

    pub(super) fn vip_update(&mut self, input: InputFrame) -> Result<()> {
        let screen = self
            .leisure
            .native_menu
            .vip
            .as_mut()
            .context("VIP viewer not open")?;
        let hover = screen
            .hover_regions
            .iter()
            .find(|(r, _)| business::contains(*r, &input))
            .map(|(_, i)| *i);
        let mut changed = hover != screen.hover;
        screen.hover = hover;
        let wheel = input.buttons_pressed & (buttons::WHEEL_UP | buttons::WHEEL_DOWN);
        if wheel != 0
            && let Some((column, grid)) = self
                .assets
                .vip_layout
                .grids
                .iter()
                .enumerate()
                .find(|(_, grid)| business::contains(grid.clip, &input))
        {
            let count = if column == 0 {
                screen.categories.len()
            } else {
                screen
                    .entries
                    .iter()
                    .filter(|entry| Some(entry.category) == screen.category)
                    .count()
            };
            let v = &mut screen.offset[column];
            *v = if wheel & buttons::WHEEL_UP != 0 {
                v.saturating_sub(1)
            } else {
                (*v + 1).min(count.saturating_sub(capacity(*grid)))
            };
            changed = true;
        }
        if changed {
            self.draw_native_menu(FreeMode)?;
        }
        Ok(())
    }

    pub(super) fn vip_click(
        &mut self,
        path: &str,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<()> {
        let screen = self
            .leisure
            .native_menu
            .vip
            .as_mut()
            .context("VIP viewer not open")?;
        if let Some((kind, tail)) = path.split_once('/')
            && matches!(kind, "vip-category" | "vip-work")
        {
            let id = tail
                .split('/')
                .next()
                .context("VIP row id")?
                .parse::<usize>()?;
            if kind == "vip-category" {
                screen.category = screen.categories.get(id).map(|row| row.0);
                screen.offset[1] = 0;
                screen.hover = None;
            } else if screen
                .entries
                .get(id)
                .is_some_and(|entry| entry.replay.enabled)
            {
                screen.selected = Some(id);
            }
            return self.draw_native_menu(FreeMode);
        }
        match path.strip_prefix(MENU) {
            Some("Cancel") => {
                self.leisure.native_menu.page = Page::Main;
                self.draw_native_menu(FreeMode)
            }
            Some("OK") => {
                let Some(entry) = screen.selected.and_then(|i| screen.entries.get(i)) else {
                    return Ok(());
                };
                let mut vm = self
                    .script
                    .tjs
                    .lock()
                    .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                let file = if entry.replay.file.contains('?') {
                    crate::tjs::retail::replace_script_name(
                        &mut vm.runtime.host_mut().state,
                        &entry.replay.file,
                    )
                    .transpose()?
                    .unwrap_or_else(|| entry.replay.file.clone())
                } else {
                    entry.replay.file.clone()
                };
                vm.eval(&format!(
                    "tf['シーン再生_ファイル名'] = {}",
                    serde_json::to_string(&file)?
                ))?;
                drop(vm);
                self.sound.stop(h, c, Kind::Bgm, 0.3)?;
                self.resume_at("scene_label_vip", op, h, c, now)
            }
            _ => Ok(()),
        }
    }
}
