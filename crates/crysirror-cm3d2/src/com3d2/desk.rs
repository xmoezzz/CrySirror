// SPDX-License-Identifier: MPL-2.0
//! The level 23 furniture list and desk model placement in office backgrounds.
use super::*;
use crate::tjs::{TjsState, retail::DeskPlacement};
use chrono::Datelike;
use std::collections::BTreeMap;

#[derive(Clone)]
struct Item {
    id: i32,
    name: String,
    prefab: String,
    flag: String,
    months: Vec<u32>,
}
#[derive(Default)]
pub(super) struct Desk {
    items: Vec<Item>,
    world: BTreeMap<i32, String>,
    dirty: bool,
    last_bg: String,
    screen: Option<Screen>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Select(usize),
    Visible,
    Season,
    Focus,
    End,
    Scroll(bool),
}
struct Screen {
    selected: Option<usize>,
    ids: Vec<usize>,
    base: Vec<Widget>,
    plate: Vec<Widget>,
    offset: usize,
    hits: Vec<([f32; 4], Action)>,
    pressed: Option<Action>,
    warning: Option<quit_dialog::QuitDialog>,
}

fn vector(value: &str) -> Result<[f32; 3]> {
    let v = value
        .split(',')
        .map(|c| c.trim().parse::<f32>())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(v.iter().all(|n| n.is_finite()), "non-finite desk transform");
    v.try_into()
        .map_err(|_| anyhow::anyhow!("invalid desk vector {value}"))
}
impl Desk {
    pub fn loaded(&self) -> bool {
        !self.items.is_empty()
    }
    pub fn clip(&self, path: &str, grid: &shop::Grid) -> Option<[f32; 4]> {
        path.starts_with("desk-row/").then_some(grid.clip)
    }
    pub(super) fn catalog(&mut self, s: &mut TjsState) -> Result<()> {
        if self.items.is_empty() || s.retail.desk.is_empty() {
            let mut items = Vec::new();
            let mut placements = s.retail.desk.clone();
            let t = s.retail.table("desk_item_detail.nei")?;
            for r in (1..t.rows).filter(|&r| t.exists(0, r)) {
                let id = t.int(0, r);
                if !s.retail.enabled("desk_item_enabled_id", id)? {
                    continue;
                }
                ensure!(
                    t.int(2, r) == 0 && t.string(4, r).is_empty(),
                    "unsupported desk category or asset bundle for item {id}"
                );
                let months = t
                    .string(6, r)
                    .split(',')
                    .filter(|v| !v.is_empty())
                    .map(|v| v.trim().parse())
                    .collect::<std::result::Result<Vec<u32>, _>>()?;
                let initial = [
                    vector(&t.string(8, r))?,
                    vector(&t.string(9, r))?,
                    vector(&t.string(10, r))?,
                ];
                placements.entry(id).or_insert(DeskPlacement {
                    visible: t.string(7, r) == "○",
                    season: !months.is_empty(),
                    transform: initial,
                });
                items.push(Item {
                    id,
                    name: t.string(1, r),
                    prefab: t.string(3, r),
                    flag: t.string(5, r),
                    months,
                });
            }
            items.sort_by_key(|i| i.id);
            self.items = items;
            s.retail.desk = placements;
        }
        Ok(())
    }
    pub fn sync(
        &mut self,
        s: &TjsState,
        assets: &mut Assets,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
    ) -> Result<()> {
        if self.items.is_empty() || !self.dirty && self.last_bg == s.bg_name {
            return Ok(());
        }
        let office = [
            "shinshitsumu",
            "shinshitsumu_night",
            "shinshitsumu_chairrot",
            "shinshitsumu_chairrot_night",
        ]
        .contains(&s.bg_name.to_lowercase().as_str());
        let selected = self.screen.as_ref().and_then(|u| u.selected);
        let month = chrono::Local::now().month();
        for (index, item) in self.items.iter().enumerate() {
            let Some(p) = s.retail.desk.get(&item.id) else {
                continue;
            };
            let visible = office
                && p.visible
                && (item.flag.is_empty() || s.player.flag(&item.flag) == 1)
                && (!p.season || item.months.contains(&month) || selected == Some(index));
            let entity = 0x35000000u32 + index as u32;
            if visible && !self.world.contains_key(&item.id) {
                let (glb, _) = club::resource(
                    &mut assets.unity,
                    op,
                    &format!("Prefab/{}", item.prefab),
                    true,
                )?;
                let asset = h.upload_model(c, entity, &glb)?;
                let [pos, rot, scale] = p.transform;
                let q = crate::chara::unity_euler(rot);
                let e = glam::Quat::from_xyzw(q.x, -q.y, -q.z, q.w).to_euler(glam::EulerRot::XYZ);
                let transform = json!({"translation":[-pos[0],pos[1],pos[2]],"rotation_degrees":[e.0.to_degrees(),e.1.to_degrees(),e.2.to_degrees()],"scale":scale});
                if let Err(error)=h.submit_engine(c,1,&serde_json::to_vec(&json!({"world":[{"op":"spawn","entity":{"id":entity,"name":item.name,"transform":transform,"geometry":{"kind":"scene","asset":asset.relative()}}}]}))?){h.release_model(c,&asset)?;return Err(error.into());}
                self.world.insert(item.id, asset.relative().to_owned());
            } else if !visible && let Some(asset) = self.world.get(&item.id) {
                h.submit_engine(
                    c,
                    1,
                    &serde_json::to_vec(&json!({"world":[{"op":"despawn","id":entity}]}))?,
                )?;
                h.release_model(
                    c,
                    &crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, asset)?,
                )?;
                self.world.remove(&item.id);
            }
        }
        self.last_bg = s.bg_name.clone();
        self.dirty = false;
        Ok(())
    }
    pub fn leave(&mut self) {
        if self.screen.take().is_some() {
            self.dirty = true;
        }
    }
    fn draw(&mut self, out: &mut Vec<Widget>, s: &TjsState, grid: shop::Grid) -> Result<()> {
        let u = self.screen.as_mut().context("missing desk window")?;
        let selected = u.selected.map(|i| &self.items[i]);
        let placement = selected.and_then(|i| s.retail.desk.get(&i.id));
        out.clear();
        u.hits.clear();
        for mut w in u.base.iter().cloned() {
            let p = &w.path;
            if p.contains("/CheckBoxSeasonalVisible/")
                && selected.is_none_or(|i| i.months.is_empty())
            {
                continue;
            }
            if p.contains("/CheckBox")
                && p.ends_with("/CheckMark")
                && !placement.is_some_and(|v| {
                    if p.contains("/CheckBoxSeasonalVisible/") {
                        v.season
                    } else {
                        v.visible
                    }
                })
            {
                continue;
            }
            if p.ends_with("/CheckBoxSeasonalVisible/BG/Text")
                && let Some(l) = &mut w.label
                && let Some(month) = selected.and_then(|i| i.months.first())
            {
                l.set_text(&format!("特定期間のみ表示する({month}月)"));
            }
            let a = if p.ends_with("/Parent/OK") {
                Some(Action::End)
            } else if p.ends_with("/CheckBoxSeasonalVisible/BG") {
                Some(Action::Season)
            } else if p.ends_with("/CheckBoxObjectVisible/BG") {
                Some(Action::Visible)
            } else if p.ends_with("/CameraFocus/Btn") {
                Some(Action::Focus)
            } else if p.ends_with("/Scroll Bar/Up") {
                Some(Action::Scroll(false))
            } else if p.ends_with("/Scroll Bar/Down") {
                Some(Action::Scroll(true))
            } else {
                None
            };
            if let (Some(r), Some(a)) = (w.collider, a) {
                u.hits.push((r, a));
            }
            out.push(w);
        }
        let visible = (grid.clip[3] / grid.step).ceil() as usize;
        u.offset = u.offset.min(u.ids.len().saturating_sub(visible));
        for (n, &i) in u.ids.iter().skip(u.offset).take(visible).enumerate() {
            let item = &self.items[i];
            for mut w in u.plate.iter().cloned() {
                if w.path.ends_with("/SelectFrame") && u.selected != Some(i) {
                    continue;
                }
                if let Some(l) = &mut w.label {
                    l.set_text(&item.name);
                }
                business::move_by(
                    &mut w,
                    grid.x,
                    grid.clip[1] + (n as f32 + 0.5) * grid.step - 540.,
                );
                w.depth += 200;
                if let Some(mut r) = w.collider {
                    r[3] = r[3].min(grid.clip[1] + grid.clip[3] - r[1]);
                    u.hits.push((r, Action::Select(i)));
                }
                w.path = format!("desk-row/{i}/{}", w.path);
                out.push(w);
            }
        }
        Ok(())
    }
}
impl Game {
    pub(super) fn open_desk(
        &mut self,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
    ) -> Result<String> {
        let mut vm = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let s = &mut vm.runtime.host_mut().state;
        self.desk.catalog(s)?;
        let ids: Vec<_> = self
            .desk
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| i.flag.is_empty() || s.player.flag(&i.flag) == 1)
            .map(|v| v.0)
            .collect();
        ensure!(
            self.assets.desk_grid.step > 0. && self.assets.desk_grid.clip[3] > 0.,
            "missing desk item grid"
        );
        self.desk.screen = Some(Screen {
            selected: ids.first().copied(),
            ids,
            base: std::mem::take(&mut self.widgets),
            plate: self.assets.scene(op, h, c, "DeskItemPlate")?,
            offset: 0,
            hits: vec![],
            pressed: None,
            warning: None,
        });
        self.desk.dirty = true;
        self.message_open = false;
        self.dialogue = None;
        self.desk
            .draw(&mut self.widgets, s, self.assets.desk_grid)?;
        Ok(if s.bg_name.is_empty() {
            daily::Period::Daytime.background().into()
        } else {
            s.bg_name.clone()
        })
    }
    pub(super) fn update_desk(
        &mut self,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
        input: InputFrame,
        released: u64,
        now: u64,
    ) -> Result<()> {
        if let Some(start) = self.leaving {
            if now.saturating_sub(start) >= 500 {
                return self.resume_script(op, h, c, now);
            }
            return Ok(());
        }
        if now.saturating_sub(self.entered) < 500 {
            return Ok(());
        }
        let u = self.desk.screen.as_mut().context("missing desk window")?;
        let mut vm = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let s = &mut vm.runtime.host_mut().state;
        if let Some(dlg) = &mut u.warning {
            if dlg.update(&self.widgets, input, released, now) == Some(quit_dialog::Choice::Ok) {
                s.player
                    .generic_flag
                    .insert("desk_customize_savedata_warning".into(), 0);
                self.widgets.truncate(dlg.first);
                u.warning = None;
                self.leaving = Some(now);
            }
            return Ok(());
        }
        let x = input.pointer_x as f32;
        let y = input.pointer_y as f32;
        let hit = u
            .hits
            .iter()
            .rev()
            .find(|(r, _)| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3])
            .map(|v| v.1);
        if input.buttons_pressed & buttons::MOUSE_LEFT != 0 {
            u.pressed = hit;
        }
        if released & buttons::MOUSE_LEFT == 0 {
            return Ok(());
        }
        let Some(a) = u.pressed.take().filter(|v| Some(*v) == hit) else {
            return Ok(());
        };
        match a {
            Action::End => {
                if s.player.flag("desk_customize_savedata_warning") == 1 {
                    let w = self.assets.scene(op, h, c, "SystemDialog")?;
                    let dlg = quit_dialog::QuitDialog::show(
                        &mut self.widgets,
                        w,
                        now,
                        "デスクトップカスタムの設定データがシステムから個々のセーブデータ依存に変更になりました\nセーブをせず終了すると、データは失われますのでご注意ください",
                    )?;
                    self.widgets.retain(|w| {
                        !(w.path.contains("SystemDialog/") && w.path.ends_with("/Cancel"))
                    });
                    u.warning = Some(dlg);
                } else {
                    s.player
                        .generic_flag
                        .insert("desk_customize_savedata_warning".into(), 0);
                    self.leaving = Some(now);
                }
                return Ok(());
            }
            Action::Select(i) => u.selected = Some(i),
            Action::Scroll(down) => {
                u.offset = if down {
                    u.offset + 1
                } else {
                    u.offset.saturating_sub(1)
                }
            }
            _ => {
                let Some(index) = u.selected else {
                    return Ok(());
                };
                let item = &self.desk.items[index];
                let p = s
                    .retail
                    .desk
                    .get_mut(&item.id)
                    .context("missing desk placement")?;
                match a {
                    Action::Visible => p.visible = !p.visible,
                    Action::Season => p.season = !p.season,
                    Action::Focus => {
                        self.adv_camera = (p.transform[0], 1.6, [-90., 4.7]);
                        let mut cam =
                            crate::bg::orbit_camera(self.adv_camera.0, self.adv_camera.2, 1.6, 35.);
                        cam["op"] = json!("set_camera");
                        h.submit_engine(c, 1, &serde_json::to_vec(&json!({"world":[cam]}))?)?;
                    }
                    _ => unreachable!(),
                }
            }
        }
        self.desk.dirty = true;
        self.desk.draw(&mut self.widgets, s, self.assets.desk_grid)
    }
}

#[test]
#[ignore = "requires installed 1.0.4 office and desk prop prefabs"]
fn installed_desk_seasonal_prop_and_return() -> Result<()> {
    let mut m = installed_menu::Menu::new()?;
    let hit = |m: &installed_menu::Menu, a| -> Result<[f32; 4]> {
        let u = m.game.desk.screen.as_ref().unwrap();
        Ok(u.hits
            .iter()
            .find(|(_, b)| *b == a)
            .with_context(|| format!("missing {a:?} in {:?}", u.hits))?
            .0)
    };
    m.open(false, "*デスクカスタマイズ", Scene::Desk)?;
    let index = m
        .game
        .desk
        .items
        .iter()
        .position(|i| i.id == 100)
        .context("missing installed Christmas tree")?;
    m.click(hit(&m, Action::Select(index))?)?;
    m.click(hit(&m, Action::Visible)?)?;
    assert!(m.game.desk.world.contains_key(&100));
    m.click(hit(&m, Action::Season)?)?;
    m.click(hit(&m, Action::Focus)?)?;
    assert_eq!(m.game.adv_camera.1, 1.6);
    m.click(hit(&m, Action::End)?)?;
    m.wait(Scene::Daily(daily::Period::Daytime))?;
    assert!(m.game.desk.world.contains_key(&100));
    m.open(false, "*デスクカスタマイズ", Scene::Desk)?;
    m.click(hit(&m, Action::Visible)?)?;
    assert!(!m.game.desk.world.contains_key(&100));
    Ok(())
}
