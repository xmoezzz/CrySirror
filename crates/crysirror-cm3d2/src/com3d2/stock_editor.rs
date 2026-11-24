// SPDX-License-Identifier: MPL-2.0
//! Retail SceneEdit creation screen. Category and slider definitions live in the mounted NEIs.
use super::*;
use crate::{
    chara::Maid,
    tjs::{TjsState, retail::prop},
};
use std::collections::BTreeMap;

const CATEGORIES: [&str; 9] = [
    "頭",
    "髪",
    "身体",
    "服装",
    "アクセサリ",
    "セット",
    "プリセット",
    "ランダム",
    "プロフィール",
];
const PANELS: [&str; 3] = [
    "ScrollPanel-Category",
    "ScrollPanel-PartsType",
    "ScrollPanel-Slider",
];

#[derive(Clone, Default)]
pub(super) struct Grid {
    pub clip: [f32; 4],
    pub step: f32,
}

#[derive(Clone)]
struct Part {
    group: String,
    title: String,
    items: Vec<(String, String, f32)>,
    sliders: Vec<(&'static str, String)>,
}

#[derive(Clone)]
enum Action {
    Category(usize),
    Part(usize),
    Slider(&'static str),
    Wear(usize),
    Finish,
    Family,
    First,
}
#[derive(Default)]
pub(super) struct StockEditor {
    groups: BTreeMap<String, Vec<Part>>,
    pub grids: [Grid; 3],
    selected: [Option<usize>; 2],
    pub naming: bool,
    pub confirming: Option<quit_dialog::QuitDialog>,
    pub pose: (String, String),
    repeat_pose: Option<u64>,
    base: usize,
    templates: [Vec<Widget>; 2],
    actions: Vec<(Action, [f32; 4])>,
    pressed: Option<usize>,
    slider: Option<(usize, [f32; 4])>,
    scroll: [f32; 3],
    drag: Option<(usize, f32, f32)>,
    pub modal_widgets: Vec<Widget>,
}

fn inside([x, y, w, h]: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= x && p[1] >= y && p[0] < x + w && p[1] < y + h
}
fn clip(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let x = a[0].max(b[0]);
    let y = a[1].max(b[1]);
    [
        x,
        y,
        ((a[0] + a[2]).min(b[0] + b[2]) - x).max(0.),
        ((a[1] + a[3]).min(b[1] + b[3]) - y).max(0.),
    ]
}
fn slider_value(m: &Maid, n: &str) -> Result<i32> {
    let value = prop(m, n)?.value;
    if n == "MuneL" && value == 0 {
        let small = prop(m, "MuneS")?;
        return Ok(
            (-30. * (small.value - small.min) as f32 / (small.max - small.min) as f32).round()
                as i32,
        );
    }
    Ok(value)
}
impl StockEditor {
    pub fn open(
        &mut self,
        state: &mut TjsState,
        tag: &BTreeMap<String, String>,
        now: u64,
    ) -> Result<String> {
        ensure!(
            !tag.contains_key("vr_mode")
                && (tag.contains_key("main") || tag.contains_key("original")),
            "unsupported SceneEdit mode"
        );
        let id = state
            .active_maids
            .first()
            .copied()
            .flatten()
            .context("SceneEdit needs active stock maid 0")?;
        ensure!(
            state.retail.stock_models.contains_key(&id),
            "SceneEdit stock model missing"
        );
        let t = state.retail.table("edit_category_define.nei")?;
        self.groups.clear();
        let mut parts = BTreeMap::new();
        for r in 1..t.rows {
            if !t.exists(0, r) || t.string(2, r) == "Color" {
                continue;
            }
            let cat = t.string(1, r);
            let group = t.string(3, r);
            let title = t.string(4, r);
            let (label, slider_name) = title.split_once('/').unwrap_or((&title, ""));
            let groups = self.groups.entry(cat.clone()).or_default();
            let i = groups
                .iter()
                .position(|v| v.group == group)
                .unwrap_or(groups.len());
            if i == groups.len() {
                groups.push(Part {
                    group,
                    title: label.to_owned(),
                    items: vec![],
                    sliders: vec![],
                });
            }
            if t.string(2, r) != "Slider" {
                parts.insert(t.string(0, r).to_lowercase(), (cat, i));
            }
            if t.string(2, r) == "Slider" {
                let name = t.string(0, r);
                let name = *crate::tjs::retail::RETAIL_MPN
                    .iter()
                    .find(|n| **n == name)
                    .context("unknown editor slider MPN")?;
                groups[i].sliders.push((name, slider_name.to_owned()));
            }
        }
        state.retail.index_parts()?;
        let (files, storage) = state.retail.editor_files()?;
        for file in files.names_with_ext(".menu") {
            if file.contains("_zurashi")
                || file.contains("_mekure")
                || !state.retail.editor_owns(&file)
            {
                continue;
            }
            let item = crate::menu::MenuFile::parse(&files.read(storage.as_ref(), &file)?)
                .with_context(|| format!("editor menu {file}"))?;
            let Some((category, group)) = parts.get(&item.category.to_lowercase()) else {
                continue;
            };
            if item
                .first("メニューフォルダ")
                .is_some_and(|v| v.eq_ignore_ascii_case("man"))
            {
                continue;
            }
            let Some(icon) = item.first("icons").or_else(|| item.first("icon")) else {
                continue;
            };
            if !files.exists(icon) {
                continue;
            }
            let priority = item
                .first("priority")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("0")
                .parse::<f32>()?;
            let title = item.first("name").unwrap_or(&item.name).to_owned();
            self.groups.get_mut(category).unwrap()[*group]
                .items
                .push((file, title, priority));
        }
        for groups in self.groups.values_mut() {
            groups.retain(|p| !p.sliders.is_empty() || !p.items.is_empty());
            for p in groups {
                p.items.sort_by(|a, b| {
                    a.2.total_cmp(&b.2)
                        .then_with(|| a.1.cmp(&b.1))
                        .then_with(|| a.0.cmp(&b.0))
                });
            }
        }
        let row = |st: &mut crate::tjs::retail::State,
                   name: &str,
                   legacy: Option<bool>|
         -> Result<(std::sync::Arc<crate::nei::Table>, usize)> {
            let table = st.table(&format!("{name}.nei"))?;
            let mut sorted = (1..table.rows)
                .filter(|&r| table.exists(0, r))
                .collect::<Vec<_>>();
            sorted.sort_by_key(|&r| table.int(0, r));
            for r in sorted {
                if legacy.is_none_or(|v| v == (table.string(5, r) == "〇"))
                    && st.enabled(&format!("{name}_enabled_list"), table.int(0, r))?
                {
                    return Ok((table, r));
                }
            }
            bail!("no enabled entry in {name}")
        };
        let legacy = state.retail.compatibility;
        let (bg, r) = row(&mut state.retail, "edit_bg", Some(legacy))?;
        ensure!(
            bg.exists(2, r)
                && bg
                    .string(6, r)
                    .trim_matches(['(', ')'])
                    .split(',')
                    .all(|v| v.trim().parse::<f32>() == Ok(0.))
                && bg.real(7, r) == 0.,
            "unsupported editor background transform"
        );
        let (pose, p) = row(&mut state.retail, "edit_pose", None)?;
        self.pose = (pose.string(3, p), pose.string(4, p));
        self.repeat_pose = Some(now + 500);
        self.selected = [None, None];
        self.naming = false;
        self.confirming = None;
        self.pressed = None;
        self.slider = None;
        self.scroll = [0.; 3];
        self.drag = None;
        state.retail.edit_owned_only = true;
        Ok(bg.string(2, r))
    }

    pub fn repeat_pose(&mut self, now: u64) -> bool {
        if self.repeat_pose.is_some_and(|time| now >= time) {
            self.repeat_pose = None;
            true
        } else {
            false
        }
    }

    pub fn load_ui(
        &mut self,
        widgets: &mut Vec<Widget>,
        assets: &mut Assets,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        model: &Maid,
    ) -> Result<()> {
        self.modal_widgets = widgets
            .extract_if(.., |w| w.path.contains("/FFNameDialog/"))
            .collect();
        self.base = widgets.len();
        self.grids = assets.stock_grids.clone();
        self.templates = [
            assets.scene(op, host, client, "StockCategoryButton")?,
            assets.scene(op, host, client, "StockSlider")?,
        ];
        self.rebuild(widgets, model)
    }

    fn entries(&self, column: usize, model: &Maid) -> Result<Vec<(Action, String, Option<i32>)>> {
        if column == 0 {
            return Ok(CATEGORIES
                .iter()
                .enumerate()
                .map(|(i, n)| (Action::Category(i), n.to_string(), None))
                .collect());
        }
        let Some(cat) = self.selected[0] else {
            return Ok(vec![]);
        };
        let groups = self
            .groups
            .get(CATEGORIES[cat])
            .context("unsupported editor category controller")?;
        if column == 1 {
            return Ok(groups
                .iter()
                .enumerate()
                .map(|(i, g)| (Action::Part(i), g.title.clone(), None))
                .collect());
        }
        let Some(i) = self.selected[1] else {
            return Ok(vec![]);
        };
        let part = &groups[i];
        if part.sliders.is_empty() {
            return Ok(part
                .items
                .iter()
                .enumerate()
                .map(|(i, (file, label, _))| {
                    (
                        Action::Wear(i),
                        if label.is_empty() {
                            file.clone()
                        } else {
                            label.clone()
                        },
                        None,
                    )
                })
                .collect());
        }
        part.sliders
            .iter()
            .map(|(n, label)| {
                Ok((
                    Action::Slider(n),
                    label.clone(),
                    Some(slider_value(model, n)?),
                ))
            })
            .collect()
    }

    fn rebuild(&mut self, widgets: &mut Vec<Widget>, model: &Maid) -> Result<()> {
        widgets.truncate(self.base);
        self.actions.clear();
        for column in 0..3 {
            let entries = self.entries(column, model)?;
            if entries.is_empty() {
                continue;
            }
            let grid = &self.grids[column];
            let bounds = grid.clip;
            ensure!(
                bounds[2] > 0. && bounds[3] > 0. && grid.step > 0.,
                "missing stock editor grid {}",
                PANELS[column]
            );

            self.scroll[column] = self.scroll[column]
                .clamp(0., (entries.len() as f32 * grid.step - bounds[3]).max(0.));
            for (i, (action, label, value)) in entries.into_iter().enumerate() {
                let y = bounds[1] + (i as f32 + 0.5) * grid.step - self.scroll[column];
                if y + grid.step / 2. < bounds[1] || y - grid.step / 2. >= bounds[1] + bounds[3] {
                    continue;
                }
                let delta = [bounds[0] + bounds[2] / 2. - 960., y - 540.];
                let index = self.actions.len();
                let mut hit = None;
                let template = &self.templates[usize::from(matches!(action, Action::Slider(_)))];
                for mut w in template.iter().cloned() {
                    if w.path.ends_with("/Frame") || w.path.ends_with("/SelectCursor") {
                        continue;
                    }
                    if let Some(l) = &mut w.label {
                        if w.path.ends_with("/Number/Value") {
                            l.set_text(&value.unwrap_or(0).to_string());
                        } else {
                            l.set_text(&label);
                        }
                    }
                    w.rect[0] += delta[0];
                    w.rect[1] += delta[1];
                    if let Some(mut r) = w.collider {
                        r[0] += delta[0];
                        r[1] += delta[1];
                        hit = Some(clip(r, bounds));
                    }
                    w.path = format!("stock-grid/{column}/{index}/{}", w.path);
                    w.depth += 100;
                    widgets.push(w);
                }
                let r = hit.unwrap_or([bounds[0], y - grid.step / 2., bounds[2], grid.step]);
                self.actions.push((action, clip(r, bounds)));
            }
        }
        for w in &widgets[..self.base] {
            if w.path == "UI Root/OkCancel/Ok"
                && let Some(r) = w.collider
            {
                self.actions.push((Action::Finish, r));
            }
        }
        if self.naming {
            self.actions.clear();
            for mut w in self.modal_widgets.iter().cloned() {
                if w.path.ends_with("/SelectCursor") {
                    continue;
                }
                if let Some(label) = &mut w.label {
                    label.set_text(if w.path.ends_with("/Message") {
                        "名前と苗字のどちらで呼びますか？"
                    } else if w.path.contains("/SelectA/") {
                        "苗字で呼ぶ"
                    } else {
                        "名前で呼ぶ"
                    });
                }
                if let Some(r) = w.collider {
                    if w.path.ends_with("/SelectA") {
                        self.actions.push((Action::Family, r))
                    }
                    if w.path.ends_with("/SelectB") {
                        self.actions.push((Action::First, r))
                    }
                }
                w.depth += 1000;
                widgets.push(w);
            }
        }
        Ok(())
    }

    pub fn diagnostics(&self, s: &TjsState) -> serde_json::Value {
        let group = self.selected[0].and_then(|i| self.groups.get(CATEGORIES[i]));
        let hits = self
            .actions
            .iter()
            .map(|(action, rect)| {
                let key = match action {
                    Action::Category(i) => CATEGORIES[*i],
                    Action::Part(i) => &group.unwrap()[*i].title,
                    Action::Slider(name) => name,
                    Action::Wear(i) => &group.unwrap()[self.selected[1].unwrap()].items[*i].0,
                    Action::Finish => "OK",
                    Action::Family => "family",
                    Action::First => "first",
                };
                (key, json!(rect))
            })
            .collect::<BTreeMap<_, _>>();
        let worn = s
            .active_maids
            .first()
            .copied()
            .flatten()
            .and_then(|id| s.bodies.get(&id))
            .map(|body| &body.menu_files);
        json!({"hits":hits,"worn":worn,"height":s.maid(0).map(|m|m.body.height),"first_name_call":s.maid(0).map(|m|m.is_first_name_call),"confirm":self.confirming.is_some()})
    }
    pub fn clip(&self, path: &str) -> Option<[f32; 4]> {
        let col = path
            .strip_prefix("stock-grid/")?
            .split('/')
            .next()?
            .parse::<usize>()
            .ok()?;
        Some(self.grids.get(col)?.clip)
    }
    pub fn update(
        &mut self,
        state: &mut TjsState,
        widgets: &mut Vec<Widget>,
        input: InputFrame,
        released: u64,
    ) -> Result<bool> {
        let id = state
            .active_maids
            .first()
            .copied()
            .flatten()
            .context("lost editor character")?;
        let point = [input.pointer_x as f32, input.pointer_y as f32];
        let hit = self.actions.iter().position(|(_, r)| inside(*r, point));
        if input.buttons_pressed & buttons::MOUSE_LEFT != 0 {
            self.pressed = hit;
            self.slider = hit.and_then(|i| {
                matches!(self.actions[i].0, Action::Slider(_)).then_some((i, self.actions[i].1))
            });
            if !self.naming && self.slider.is_none() {
                self.drag = self
                    .grids
                    .iter()
                    .position(|g| inside(g.clip, point))
                    .map(|c| (c, point[1], self.scroll[c]));
            }
        }
        if let Some((i, r)) = self.slider
            && (input.buttons_down | released) & buttons::MOUSE_LEFT != 0
        {
            let Action::Slider(name) = self.actions[i].0 else {
                bail!("editor slider changed while held")
            };
            let value = ((point[0] - r[0]) / r[2].max(1.)).clamp(0., 1.);
            crate::tjs::retail::edit_slider(state, id, name, value)?;
            self.rebuild(widgets, &state.retail.stock_models[&id])?;
        } else if let Some((col, y, initial)) = self.drag
            && (point[1] - y).abs() > 4.
        {
            self.scroll[col] = initial + y - point[1];
            self.pressed = None;
            self.rebuild(widgets, &state.retail.stock_models[&id])?;
        }
        if released & buttons::MOUSE_LEFT == 0 {
            return Ok(false);
        }
        self.drag = None;
        self.slider = None;
        if let Some(i) = self.pressed.take()
            && Some(i) == hit
        {
            match self.actions[i].0 {
                Action::Category(c) => {
                    self.selected = [Some(c), None];
                    self.scroll[1..].fill(0.);
                }
                Action::Part(i) => {
                    self.selected[1] = Some(i);
                    self.scroll[2] = 0.;
                }
                Action::Wear(index) => {
                    let group = &self.groups
                        [CATEGORIES[self.selected[0].context("missing edit category")?]]
                        [self.selected[1].context("missing item group")?];
                    crate::tjs::retail::edit_wear(state, id, &group.items[index].0)?;
                }
                Action::Finish => self.naming = true,
                Action::Family | Action::First => {
                    state
                        .maids
                        .get_mut(&id)
                        .context("missing editor status")?
                        .is_first_name_call = matches!(self.actions[i].0, Action::First);
                    self.naming = false;
                    self.rebuild(widgets, &state.retail.stock_models[&id])?;
                    return Ok(true);
                }
                Action::Slider(_) => return Ok(false),
            }
            self.rebuild(widgets, &state.retail.stock_models[&id])?;
        }
        Ok(false)
    }
}
