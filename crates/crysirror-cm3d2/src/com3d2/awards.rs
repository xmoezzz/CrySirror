// SPDX-License-Identifier: MPL-2.0
//! Standalone level 10 SceneTrophyManager / ScreenTrophyMain.
use super::*;
use crate::tjs::TjsState;
use crysirror_unity::reader::Reader;

const CATEGORIES: [&str; 3] = ["ヒロイン", "システム", "エクストラ"];
const CARD: &str = "UI Root/Parent/Card/";
type Rect = [f32; 4];

#[derive(Default, Clone, Copy)]
pub(super) struct Layout {
    category: shop::Grid,
    pub list: shop::Grid,
    fade_ms: u64,
}
impl Layout {
    pub fn read(&mut self, class: &str, path: &str, offset: [f32; 3], data: &[u8]) -> Result<()> {
        if path.contains("/MainCategoryViewer/Content") {
            self.category.read(class, path, offset, data)?;
        }
        if path.contains("/TrophyList/Content") {
            self.list.read(class, path, offset, data)?;
        }
        if class == "SceneTrophyCardFade" {
            let mut r = Reader::new(data);
            r.seek(32)?;
            self.fade_ms = (r.f32()? * 1000.).round() as u64;
        }
        Ok(())
    }
}

struct Entry {
    id: i32,
    group: usize,
    rarity: usize,
    owned: bool,
    title: String,
    points: i32,
    info: String,
    bonus: String,
    mini: String,
    card: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Category(usize),
    Card(i32),
    Dismiss,
    Exit,
    Page(usize),
    CloseHelp,
    Scroll(bool),
    Thumb,
}
struct Zoom {
    from: [f32; 2],
    start: u64,
    closing: bool,
}
pub(super) struct Screen {
    pub layout: Layout,
    entries: Vec<Entry>,
    counts: [usize; 3],
    category: usize,
    offset: f32,
    zoom: Option<Zoom>,
    base: Vec<Widget>,
    card: Vec<Widget>,
    buttons: Vec<Widget>,
    units: [Vec<Widget>; 3],
    hits: Vec<(Action, Rect)>,
    pressed: Option<Action>,
    drag: Option<(f32, f32, bool)>,
    pages: Vec<(String, String)>,
    page: usize,
    help_start: usize,
    help_closing: Option<u64>,
    fade_checks: u8,
    pub(super) fade_started: Option<u64>,
}
fn contains([x, y, w, h]: Rect, p: [f32; 2]) -> bool {
    p[0] >= x && p[0] < x + w && p[1] >= y && p[1] < y + h
}
fn center([x, y, w, h]: Rect) -> [f32; 2] {
    [x + w / 2., y + h / 2.]
}
impl Screen {
    fn open(
        s: &mut TjsState,
        base: Vec<Widget>,
        assets: &mut Assets,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
    ) -> Result<Self> {
        let table = s.retail.table("trophy_list.nei")?;
        let mut entries = vec![];
        let mut counts = [0; 3];
        let mut owned = [0; 3];
        let mut points = 0;
        for r in (1..table.rows).filter(|&r| table.exists(0, r)) {
            let id = table.int(0, r);
            if !s.retail.enabled("trophy_enabled_list", id)? {
                continue;
            }
            let group = CATEGORIES
                .iter()
                .position(|&g| g == table.string(4, r))
                .unwrap_or(2);
            let have = s.player.have_trophy.contains(&id);
            let pts = table.int(3, r);
            counts[group] += 1;
            if have {
                owned[group] += 1;
                points += pts;
            }
            entries.push(Entry {
                id,
                group,
                owned: have,
                points: pts,
                rarity: match table.int(1, r) {
                    2 => 1,
                    3 => 2,
                    _ => 0,
                },
                title: if !have && table.string(10, r) == "〇" {
                    "????????".into()
                } else {
                    table.string(2, r)
                },
                info: table.string(7, r),
                bonus: table.string(8, r),
                mini: table.string(5, r),
                card: table.string(6, r),
            });
        }
        entries.sort_by_key(|e| e.id);
        let category = counts
            .iter()
            .position(|&n| n > 0)
            .context("no enabled trophies")?;
        let (card, mut base): (Vec<_>, Vec<_>) =
            base.into_iter().partition(|w| w.path.starts_with(CARD));
        if counts[2] == 0 {
            base.retain(|w| !w.path.contains("/TrophyNums/Extra"));
        }
        for w in &mut base {
            if counts[2] == 0 && w.path.contains("/InfoPanel/BG/Heroine") {
                business::move_by(w, 0., -38.);
            }
            if counts[2] == 0 && w.path.ends_with("/InfoPanel/BG") {
                w.rect[3] -= 38.;
            }
            let Some(l) = &mut w.label else { continue };
            for (i, g) in ["Heroine", "System", "Extra"].iter().enumerate() {
                if w.path.ends_with(&format!("/TrophyNums/{g}/Value")) {
                    l.set_text(&format!("{}/{}", owned[i], counts[i]));
                }
            }
            if w.path.ends_with("/InfoPanel/BG/Heroine/Value") {
                l.set_text(&points.to_string());
            }
        }
        let layout = assets.trophy_layout;
        ensure!(
            layout.fade_ms > 0
                && layout.category.step > 0.
                && layout.list.step > 0.
                && layout.list.clip[3] >= layout.list.step,
            "invalid level 10 grids or card fade"
        );
        let mut load = |name: &str| assets.scene(op, host, c, name);
        let buttons = load("TrainingCategory")?;
        let units = [
            load("TrophyInfo/bronze")?,
            load("TrophyInfo/silver")?,
            load("TrophyInfo/gold")?,
        ];
        let mut pages = vec![];
        if s.player.flag("__チュートリアルフラグ_SceneTrophy") <= 0 {
            let t = s.retail.table("tutorial_list.nei")?;
            if let Some(r) = (1..t.rows).find(|&r| t.string(0, r) == "SceneTrophy") {
                pages = (2..t.columns)
                    .step_by(2)
                    .filter(|&col| t.exists(col, r))
                    .map(|col| (t.string(col, r), t.string(col + 1, r)))
                    .collect();
            }
        }
        Ok(Self {
            entries,
            counts,
            category,
            layout,
            base,
            card,
            buttons,
            units,
            hits: vec![],
            zoom: None,
            pressed: None,
            drag: None,
            offset: 0.,
            pages,
            page: 0,
            help_start: 0,
            help_closing: None,
            fade_checks: 1,
            fade_started: None,
        })
    }
    fn max_offset(&self) -> f32 {
        (self.counts[self.category] as f32 * self.layout.list.step - self.layout.list.clip[3])
            .max(0.)
    }
    fn widgets(
        &mut self,
        ws: &mut Vec<Widget>,
        a: &mut Assets,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
        files: &GameFiles,
    ) -> Result<()> {
        ws.clone_from(&self.base);
        self.hits.clear();
        let cats: Vec<_> = (0..3).filter(|&i| self.counts[i] > 0).collect();
        let grid = self.layout.category;
        let middle = grid.clip[1] + grid.clip[3] / 2.;
        let cy = |i: usize| middle + (i as f32 - (cats.len() - 1) as f32 / 2.) * grid.step;
        let max = self.max_offset();
        for w in ws.iter_mut() {
            if w.path.ends_with("/MainCategoryViewer/Arrow") {
                let y = cy(cats.iter().position(|&v| v == self.category).unwrap());
                w.rect[1] = y - w.rect[3] / 2.;
            }
            let Some(rect) = w.collider else { continue };
            if w.path.ends_with("/MainItems/OK") {
                self.hits.push((Action::Exit, rect));
            }
            if w.path.contains("/TrophyList/Scroll Bar/") {
                if w.path.ends_with("/Thumb") {
                    let clip = self.layout.list.clip;
                    let height = clip[3] * clip[3] / (max + clip[3]);
                    w.rect[1] = clip[1] + (clip[3] - height) * self.offset / max.max(1.);
                    w.rect[3] = height;
                    w.collider = Some(w.rect);
                    self.hits.push((Action::Thumb, w.rect));
                } else if w.path.ends_with("/Up") || w.path.ends_with("/Down") {
                    self.hits
                        .push((Action::Scroll(w.path.ends_with("/Down")), rect));
                }
            }
        }
        for (n, &cat) in cats.iter().enumerate() {
            for mut w in self.buttons.iter().cloned() {
                business::move_by(&mut w, grid.x, cy(n) - 540.);
                w.depth += 100;
                if let Some(l) = &mut w.label {
                    l.set_text(CATEGORIES[cat]);
                }
                if let Some(r) = w.collider {
                    self.hits.push((Action::Category(cat), r));
                }
                ws.push(w);
            }
        }
        let grid = self.layout.list;
        for (i, e) in self
            .entries
            .iter()
            .filter(|e| e.group == self.category)
            .enumerate()
        {
            let y = grid.clip[1] + (i as f32 + 0.5) * grid.step - self.offset;
            if y + grid.step / 2. <= grid.clip[1]
                || y - grid.step / 2. >= grid.clip[1] + grid.clip[3]
            {
                continue;
            }
            for mut w in self.units[e.rarity].iter().cloned() {
                let path = w.path.clone();
                let is_tex = path.ends_with("/Card/Tex");
                if is_tex && (!e.owned || e.mini.is_empty()) {
                    continue;
                }
                if is_tex {
                    w.asset = a.schedule_icon(op, h, c, files, &e.mini)?;
                }
                if let Some(l) = &mut w.label {
                    if path.ends_with("/TitleLabel") {
                        l.set_text(&e.title);
                    }
                    if path.ends_with("/PointLabel") {
                        l.set_text(&e.points.to_string());
                    }
                    if path.ends_with("/CommentLabel") {
                        l.set_text(&e.info);
                    }
                    if path.ends_with("/GetItemLabel") {
                        l.set_text(&e.bonus);
                    }
                    if path.ends_with("/TitleLabel") || path.ends_with("/PointLabel") {
                        w.tint = if e.owned {
                            [64, 64, 64, 255]
                        } else {
                            [128, 128, 128, 255]
                        };
                    }
                }
                if path.ends_with("/TrophyImage/Image") {
                    w.tint[3] = if e.owned { 255 } else { 77 };
                }
                if path.ends_with("/BG") {
                    w.tint[3] = if e.owned { 255 } else { 204 };
                }
                business::move_by(&mut w, grid.x, y - 540.);
                if e.owned
                    && path.ends_with("/Card")
                    && let Some(mut r) = w.collider
                {
                    let bottom = (r[1] + r[3]).min(grid.clip[1] + grid.clip[3]);
                    r[1] = r[1].max(grid.clip[1]);
                    r[3] = bottom - r[1];
                    self.hits.push((Action::Card(e.id), r));
                }
                w.depth += 100;
                w.path = format!("trophy-row/{}/{path}", e.id);
                ws.push(w);
            }
        }
        self.help_start = ws.len();
        if !self.pages.is_empty() {
            a.tutorial(op, h, c, &self.pages, self.page, files, ws)?;
            self.hits.clear();
            for w in &ws[self.help_start..] {
                if let Some(r) = w.collider {
                    if w.path.ends_with("/Button OK") {
                        self.hits.push((Action::CloseHelp, r));
                    }
                    if let Some((_, page)) = w.path.split_once("/page-") {
                        self.hits
                            .push((Action::Page(page.split('/').next().unwrap().parse()?), r));
                    }
                }
            }
        }
        Ok(())
    }
    pub fn opacity_and_rect(&self, path: &str, rect: &mut Rect, now: u64, opacity: u8) -> u8 {
        if path.starts_with("/Canvas Tutorial Panel") {
            let f = self.help_closing.map_or(1., |start| {
                1. - (now.saturating_sub(start) as f32 / 500.).min(1.)
            });
            return (f * opacity as f32).round() as u8;
        }
        let Some(z) = &self.zoom else {
            return opacity;
        };
        let t = (now.saturating_sub(z.start) as f32 / self.layout.fade_ms as f32).min(1.);
        let v = 0.5 - 0.5 * (std::f32::consts::PI * t).cos();
        let v = if z.closing { 1. - v } else { v };
        if path.starts_with(CARD) {
            if path.ends_with("/CartSprite") {
                let end = center(*rect);
                let scale = 0.12 + 0.88 * v * v * (3. - 2. * v);
                rect[2] *= scale;
                rect[3] *= scale;
                rect[0] = z.from[0] + (end[0] - z.from[0]) * v - rect[2] / 2.;
                rect[1] = z.from[1] + (end[1] - z.from[1]) * v - rect[3] / 2.;
            }
            (v * opacity as f32).round() as u8
        } else {
            ((1. - v) * opacity as f32).round() as u8
        }
    }
}
impl Game {
    pub(super) fn open_awards(
        &mut self,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
    ) -> Result<()> {
        self.message_open = false;
        self.dialogue = None;
        self.camera_control = true;
        self.main_light = MainLight::default();
        self.adv_camera = ([0., 1.5, 0.], 2., [180., 10.]);
        self.camera_move = None;
        let mut camera =
            crate::bg::orbit_camera(self.adv_camera.0, self.adv_camera.2, self.adv_camera.1, 35.);
        camera["op"] = json!("set_camera");
        let light = json!({"op":"set_environment","environment":self.main_light.environment(&self.settings)});
        h.submit_engine(c, 1, &serde_json::to_vec(&json!({"world":[camera,light]}))?)?;
        let mut vm = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let s = &mut vm.runtime.host_mut().state;
        self.actors.reset_positions();
        let mut screen = Screen::open(
            s,
            std::mem::take(&mut self.widgets),
            &mut self.assets,
            op,
            h,
            c,
        )?;
        screen.widgets(&mut self.widgets, &mut self.assets, op, h, c, &self.files)?;
        if !screen.pages.is_empty() {
            s.player
                .generic_flag
                .insert("__チュートリアルフラグ_SceneTrophy".into(), 1);
        }
        self.awards = Some(screen);
        Ok(())
    }
    pub(super) fn update_awards(
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
                self.resume_script(op, h, c, now)?;
            }
            return Ok(());
        }
        let screen = self.awards.as_mut().context("missing trophy screen")?;
        if screen.fade_started.is_none() {
            screen.fade_checks += 1;
            if screen.fade_checks < 3 {
                return Ok(());
            }
            screen.fade_started = Some(now);
        }
        if now.saturating_sub(screen.fade_started.unwrap()) < 500 {
            return Ok(());
        }
        if let Some(start) = screen.help_closing {
            if now.saturating_sub(start) < 500 {
                return Ok(());
            }
            screen.help_closing = None;
            screen.pages.clear();
            return screen.widgets(&mut self.widgets, &mut self.assets, op, h, c, &self.files);
        }
        if let Some(z) = &screen.zoom {
            if now.saturating_sub(z.start) < screen.layout.fade_ms {
                return Ok(());
            }
            if z.closing {
                screen.zoom = None;
                return screen.widgets(&mut self.widgets, &mut self.assets, op, h, c, &self.files);
            }
        }
        let p = [input.pointer_x as f32, input.pointer_y as f32];
        let hit = screen
            .hits
            .iter()
            .rev()
            .find(|(_, r)| contains(*r, p))
            .map(|v| v.0);
        if input.buttons_pressed & buttons::MOUSE_LEFT != 0 {
            screen.pressed = hit;
            if screen.zoom.is_none()
                && screen.pages.is_empty()
                && (contains(screen.layout.list.clip, p) || hit == Some(Action::Thumb))
            {
                screen.drag = Some((p[1], screen.offset, hit == Some(Action::Thumb)));
            }
        }
        if let Some((y, offset, thumb)) = screen.drag
            && (input.buttons_down | released) & buttons::MOUSE_LEFT != 0
            && (y - p[1]).abs() > 3.
        {
            let max = screen.max_offset();
            let dy = if thumb {
                (p[1] - y) * (max + screen.layout.list.clip[3]) / screen.layout.list.clip[3]
            } else {
                y - p[1]
            };
            screen.offset = (offset + dy).clamp(0., max);
            screen.pressed = None;
            screen.widgets(&mut self.widgets, &mut self.assets, op, h, c, &self.files)?;
        }
        if released & buttons::MOUSE_LEFT == 0 {
            return Ok(());
        }
        screen.drag = None;
        let Some(action) = screen.pressed.take().filter(|a| Some(*a) == hit) else {
            return Ok(());
        };
        match action {
            Action::Page(n) => screen.page = n,
            Action::CloseHelp => {
                screen.help_closing = Some(now);
                return Ok(());
            }
            Action::Scroll(down) => {
                screen.offset = (screen.offset
                    + if down {
                        screen.layout.list.step
                    } else {
                        -screen.layout.list.step
                    })
                .clamp(0., screen.max_offset())
            }
            Action::Category(n) => {
                screen.category = n;
                screen.offset = 0.;
            }
            Action::Thumb => return Ok(()),
            Action::Exit => {
                self.leaving = Some(now);
                return Ok(());
            }
            Action::Card(id) => {
                let e = screen
                    .entries
                    .iter()
                    .find(|e| e.id == id && e.owned)
                    .context("unowned trophy card")?;
                let source = format!("trophy-row/{id}/TrophyInfo/Card/Tex");
                let original = self
                    .widgets
                    .iter()
                    .find(|w| w.path == source)
                    .map(|w| w.rect)
                    .or_else(|| screen.hits.iter().find(|v| v.0 == action).map(|v| v.1))
                    .context("missing trophy card origin")?;
                screen.hits.clear();
                for mut w in screen.card.iter().cloned() {
                    if w.path.ends_with("/CartSprite") {
                        if e.card.is_empty() {
                            continue;
                        }
                        w.asset = self.assets.schedule_icon(op, h, c, &self.files, &e.card)?;
                    }
                    if let Some(r) = w.collider {
                        screen.hits.push((Action::Dismiss, r));
                    }
                    w.depth += 500;
                    self.widgets.push(w);
                }
                screen.zoom = Some(Zoom {
                    from: center(original),
                    start: now,
                    closing: false,
                });
                return Ok(());
            }
            Action::Dismiss => {
                let z = screen.zoom.as_mut().context("missing trophy card")?;
                z.start = now;
                z.closing = true;
                return Ok(());
            }
        }
        screen.widgets(&mut self.widgets, &mut self.assets, op, h, c, &self.files)
    }
}

#[test]
#[ignore = "requires the installed 1.0.4 trophy level, prefabs, scripts and textures"]
fn installed_awards_categories_cards_and_both_callers() -> Result<()> {
    let packs = ["sound", "sound2"];
    let (_tmp, mut rt, c, mut op, files, _, _) = daily::installed_panel(&packs)?;
    let mut vm = Vm {
        linked_cm3d2: None,
        client: c,
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
    let game = vm.game.as_mut().unwrap();
    {
        let mut t = game.script.tjs.lock().unwrap();
        let s = &mut t.runtime.host_mut().state;
        s.player.schedule_slot.resize_with(40, Default::default);
        crate::tjs::retail::operations::start_day(s)?;
        s.player.have_trophy.add(10);
    }
    for (file, period) in [
        ("daytimemain.ks", daily::Period::Daytime),
        ("nightmain.ks", daily::Period::Night),
    ] {
        let label = "*トロフィー";
        game.script.load(&files, op.storage.as_ref(), file, label)?;
        game.script.enabled = true;
        game.script.exec();
        game.enter(Scene::Script, &mut op, &mut rt.host, c, 0)?;
        for _ in 0..32 {
            rt.tick_control(250, InputFrame::default())?;
            game.update(&mut rt.host, c, &mut op, InputFrame::default())?;
            if game.scene == Scene::Awards {
                break;
            }
        }
        assert_eq!(
            game.scene,
            Scene::Awards,
            "{file} {}",
            game.script.position()
        );

        let tick = |g: &mut Game, rt: &mut crysirror_runtime::Runtime, op: &mut Opener, ms: u64| {
            rt.tick_control(ms, InputFrame::default())?;
            g.update(&mut rt.host, c, op, InputFrame::default())
        };
        let click = |g: &mut Game,
                     rt: &mut crysirror_runtime::Runtime,
                     op: &mut Opener,
                     action: Action|
         -> Result<()> {
            let a = g.awards.as_ref().unwrap();
            let r = a
                .hits
                .iter()
                .find(|v| v.0 == action)
                .with_context(|| format!("missing {action:?} in {:?}", a.hits))?
                .1;
            let [x, y] = center(r);
            let input = InputFrame {
                buttons_down: buttons::MOUSE_LEFT,
                buttons_pressed: buttons::MOUSE_LEFT,
                pointer_x: x as i32,
                pointer_y: y as i32,
                ..Default::default()
            };
            g.update(&mut rt.host, c, op, input)?;
            g.update(
                &mut rt.host,
                c,
                op,
                InputFrame {
                    buttons_down: 0,
                    buttons_pressed: 0,
                    ..input
                },
            )
        };
        assert_eq!(game.adv_camera, ([0., 1.5, 0.], 2., [180., 10.]));
        tick(game, &mut rt, &mut op, 1)?;
        assert!(game.awards.as_ref().unwrap().fade_started.is_none());
        tick(game, &mut rt, &mut op, 1)?;
        tick(game, &mut rt, &mut op, 500)?;
        if period == daily::Period::Daytime {
            let a = game.awards.as_ref().unwrap();
            assert_eq!(a.pages.len(), 2);
            assert!(!a.hits.iter().any(|v| v.0 == Action::Exit));
            click(game, &mut rt, &mut op, Action::Page(1))?;
            assert_eq!(game.awards.as_ref().unwrap().page, 1);
            click(game, &mut rt, &mut op, Action::CloseHelp)?;
            tick(game, &mut rt, &mut op, 500)?;
        }
        let a = game.awards.as_ref().unwrap();
        assert!(a.pages.is_empty());
        assert_eq!(a.category, 0);
        let label = |path: &str| {
            game.widgets
                .iter()
                .find(|w| w.path.ends_with(path))
                .and_then(|w| w.label.as_ref())
                .map(|l| l.text.as_str())
        };
        let first = a.entries.iter().find(|e| e.id == 10).unwrap();
        assert_eq!(
            label("/InfoPanel/BG/Heroine/Value"),
            Some(first.points.to_string().as_str())
        );
        assert_eq!(
            label("/TrophyNums/Heroine/Value"),
            Some(format!("1/{}", a.counts[0]).as_str())
        );
        assert!(a.counts[0] > 1 && a.counts[1] > 1);
        assert!(a.entries.iter().any(|e| !e.owned && e.title == "????????"));
        assert!(
            !game
                .widgets
                .iter()
                .any(|w| w.path.contains("/TrophyNums/Extra"))
        );
        assert!(
            !a.hits
                .iter()
                .any(|v| matches!(v.0,Action::Card(id) if id!=10))
        );
        click(game, &mut rt, &mut op, Action::Card(10))?;
        let a = game.awards.as_ref().unwrap();
        assert!(a.hits.iter().all(|v| v.0 == Action::Dismiss));
        let sprite = game
            .widgets
            .iter()
            .find(|w| w.path.ends_with("/CartSprite"))
            .context("missing zoomed trophy")?;
        assert!(!sprite.asset.is_empty());
        let mut r = sprite.rect;
        let start = a.zoom.as_ref().unwrap().start;
        let duration = a.layout.fade_ms;
        assert_eq!(
            a.opacity_and_rect(&sprite.path, &mut r, start + duration / 2, 255),
            128
        );
        assert!(r[2] > sprite.rect[2] * 0.12 && r[2] < sprite.rect[2]);
        tick(game, &mut rt, &mut op, duration)?;
        click(game, &mut rt, &mut op, Action::Dismiss)?;
        tick(game, &mut rt, &mut op, duration)?;
        click(game, &mut rt, &mut op, Action::Category(1))?;
        assert_eq!(game.awards.as_ref().unwrap().category, 1);
        let ids = |g: &Game| {
            g.widgets
                .iter()
                .filter(|w| w.path.starts_with("trophy-row/") && w.path.ends_with("/TitleLabel"))
                .map(|w| w.path.clone())
                .collect::<Vec<_>>()
        };
        let before = ids(game);
        click(game, &mut rt, &mut op, Action::Scroll(true))?;
        assert_ne!(before, ids(game));
        click(game, &mut rt, &mut op, Action::Exit)?;
        tick(game, &mut rt, &mut op, 500)?;
        assert_eq!(game.scene, Scene::Script);
        for _ in 0..32 {
            tick(game, &mut rt, &mut op, 250)?;
            if game.scene == Scene::Daily(period) {
                break;
            }
        }
        assert_eq!(game.scene, Scene::Daily(period));
    }
    Ok(())
}
