// SPDX-License-Identifier: MPL-2.0
//! Level 49 ScenarioSelect, including its original KAG return and event state.
use super::*;
use crate::tjs::{TjsState, retail::scenario};
use crysirror_unity::reader::Reader;

const PANELS: [&str; 3] = [
    "/ScenarioSelect/SelectPanel",
    "/PlayableInfo",
    "/CharacterSelectPanel",
];
#[derive(Clone, Default)]
pub(super) struct Layout {
    grids: [shop::Grid; 3],
    day: String,
    night: String,
    music: String,
}
impl Layout {
    pub fn read(&mut self, class: &str, path: &str, pos: [f32; 3], data: &[u8]) -> Result<()> {
        if class == "SceneScenarioSelect" {
            let mut r = Reader::new(data);
            r.seek(32)?;
            self.day = r.string()?;
            self.night = r.string()?;
            self.music = r.string()?;
        }
        if let Some(i) = PANELS.iter().position(|p| path.contains(p)) {
            self.grids[i].read(class, path, pos, data)?;
        }
        Ok(())
    }
}
type Rect = [f32; 4];
#[derive(Clone, Copy, PartialEq, Debug)]
enum Action {
    Event(usize),
    Maid(usize),
    Scroll(usize, bool),
    Thumb(usize),
    Ok,
    Cancel,
}
fn contains(r: Rect, p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[0] < r[0] + r[2] && p[1] >= r[1] && p[1] < r[1] + r[3]
}
pub(super) struct Screen {
    list: Vec<scenario::Selection>,
    selected: usize,
    maid: Option<usize>,
    layout: Layout,
    base: Vec<Widget>,
    templates: [Vec<Widget>; 3],
    offsets: [f32; 3],
    hits: Vec<(Rect, Action)>,
    pressed: Option<Action>,
    drag: Option<(usize, f32, f32, bool)>,
    confirm: bool,
}
impl Screen {
    pub fn clip(&self, path: &str) -> Option<Rect> {
        let list = path
            .strip_prefix("event-grid/")?
            .split('/')
            .next()?
            .parse::<usize>()
            .ok()?;
        Some(self.layout.grids.get(list)?.clip)
    }
    fn lengths(&self) -> [usize; 3] {
        let entry = self.list.get(self.selected);
        [
            self.list.len(),
            entry.map_or(0, |e| e.conditions.iter().filter(|s| !s.is_empty()).count()),
            entry.map_or(0, |e| e.maids.len()),
        ]
    }
    fn select(&mut self, index: usize) {
        self.selected = index;
        self.maid = self.list.get(index).and_then(|e| e.maids.first().copied());
        self.offsets[1..].fill(0.);
    }
    fn draw(
        &mut self,
        out: &mut Vec<Widget>,
        s: &TjsState,
        assets: &mut Assets,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
    ) -> Result<()> {
        out.clone_from(&self.base);
        self.hits.clear();
        let lens = self.lengths();
        let entry = self.list.get(self.selected);
        for (n, g) in self.layout.grids.iter().enumerate() {
            self.offsets[n] =
                self.offsets[n].clamp(0., (lens[n] as f32 * g.step - g.clip[3]).max(0.));
        }
        for w in out.iter_mut() {
            if w.path.ends_with("/EventContents/Label")
                && let Some(l) = &mut w.label
            {
                l.set_text(entry.map_or("", |e| &e.contents));
            }
            let Some(r) = w.collider else {
                continue;
            };
            if w.path.ends_with("/ButtonParent/OK") {
                if entry.is_some() {
                    self.hits.push((r, Action::Ok));
                } else {
                    w.tint = [128, 128, 128, 255];
                }
            } else if w.path.ends_with("/ButtonParent/Cancel") {
                self.hits.push((r, Action::Cancel));
            }
            if w.path.contains("/Scroll Bar/")
                && let Some(n) = PANELS.iter().position(|p| w.path.contains(p))
            {
                let g = self.layout.grids[n];
                if w.path.ends_with("/Up") || w.path.ends_with("/Down") {
                    self.hits
                        .push((r, Action::Scroll(n, w.path.ends_with("/Down"))));
                }
                if w.path.ends_with("/Thumb") {
                    let height = g.clip[3];
                    let full = (lens[n] as f32 * g.step).max(height);
                    w.rect[1] = g.clip[1] + self.offsets[n] * height / full;
                    w.rect[3] = height * height / full;
                    self.hits.push((w.rect, Action::Thumb(n)));
                }
            }
        }
        for n in 0..3 {
            let g = self.layout.grids[n];
            let skip = (self.offsets[n] / g.step).floor() as usize;
            for i in skip..lens[n].min(skip + (g.clip[3] / g.step).ceil() as usize + 1) {
                let y = g.clip[1] + (i as f32 + 0.5) * g.step - self.offsets[n];
                for mut w in self.templates[n].iter().cloned() {
                    let action = match n {
                        0 => {
                            let row = &self.list[i];
                            if w.path.ends_with("/SelectFrame") && i != self.selected {
                                continue;
                            }
                            if let Some(l) = &mut w.label
                                && w.path.ends_with("/Title")
                            {
                                l.set_text(&row.title);
                            }
                            if w.path.ends_with("/Icon")
                                && !assets.resource_sprite(
                                    &mut w,
                                    op,
                                    h,
                                    c,
                                    &format!("SceneScenarioSelect/Sprite/{}", row.icon),
                                )?
                            {
                                continue;
                            }
                            Some(Action::Event(i))
                        }
                        1 => {
                            let (number, cond) = entry
                                .unwrap()
                                .conditions
                                .iter()
                                .enumerate()
                                .filter(|(_, t)| !t.is_empty())
                                .nth(i)
                                .unwrap();
                            if let Some(l) = &mut w.label {
                                if w.path.ends_with("/Number") {
                                    l.set_text(&(number + 1).to_string());
                                } else if w.path.ends_with("/Text") {
                                    l.set_text(cond);
                                }
                            }
                            None
                        }
                        _ => {
                            let e = entry.unwrap();
                            let id = e.maids[i];
                            if !business::prepare_plate(
                                &mut w,
                                id,
                                e.fixed || self.maid == Some(id),
                                s,
                                assets,
                                h,
                                c,
                            )? {
                                continue;
                            }
                            (!e.fixed && w.path.ends_with("/Button")).then_some(Action::Maid(id))
                        }
                    };
                    business::move_by(&mut w, g.x, y - 540.);
                    w.depth += 200;
                    if let (Some(mut r), Some(a)) = (w.collider, action) {
                        let bottom = (r[1] + r[3]).min(g.clip[1] + g.clip[3]);
                        r[1] = r[1].max(g.clip[1]);
                        r[3] = bottom - r[1];
                        self.hits.push((r, a));
                    }
                    w.path = format!("event-grid/{n}/{i}/{}", w.path);
                    out.push(w);
                }
            }
        }
        Ok(())
    }
}

impl Game {
    pub(super) fn open_events(
        &mut self,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
    ) -> Result<String> {
        self.message_open = false;
        self.dialogue = None;
        self.adv_camera = ([0., 1.5, 0.], 2., [180., 10.]);
        self.camera_control = true;
        self.camera_move = None;
        let mut vm = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let s = &mut vm.runtime.host_mut().state;
        self.actors.reset_for_day(s);
        s.active_maids.fill(None);
        let layout = self.assets.scenario_layout.clone();
        ensure!(
            layout.grids.iter().all(|g| g.step > 0. && g.clip[3] > 0.) && !layout.music.is_empty(),
            "missing level 49 configuration"
        );
        let bg = if s.player.flag("時間帯") == 3 {
            &layout.night
        } else {
            &layout.day
        };
        let bg = bg.clone();
        self.sound.stop(h, c, Kind::Voice, 0.)?;
        self.sound.stop(h, c, Kind::Se, 0.)?;
        self.sound
            .play(h, c, &self.files, Kind::Bgm, &layout.music, 0., true)?;
        let mut load = |name: &str| self.assets.scene(op, h, c, name);
        let mut screen = Screen {
            list: scenario::selections(s)?,
            layout,
            base: std::mem::take(&mut self.widgets),
            templates: [
                load("ScenarioPlate")?,
                load("ScenarioCondition")?,
                load("PickerPlate")?,
            ],
            selected: 0,
            maid: None,
            hits: vec![],
            offsets: [0.; 3],
            drag: None,
            pressed: None,
            confirm: false,
        };
        screen.select(0);
        screen.draw(&mut self.widgets, s, &mut self.assets, op, h, c)?;
        self.events = Some(screen);
        Ok(bg)
    }
    pub(super) fn update_events(
        &mut self,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
        input: InputFrame,
        released: u64,
        now: u64,
    ) -> Result<()> {
        let screen = self.events.as_mut().context("missing scenario selector")?;
        if let Some(start) = self.leaving {
            if now.saturating_sub(start) < 500 {
                return Ok(());
            }
            let key = if screen.confirm {
                let mut vm = self
                    .script
                    .tjs
                    .lock()
                    .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                screen.list[screen.selected].play(&mut vm, screen.maid)?;
                "label"
            } else {
                "cancel_label"
            };
            return self.resume_at(key, op, h, c, now);
        }
        if now.saturating_sub(self.entered) < 500 {
            return Ok(());
        }
        let point = [input.pointer_x as f32, input.pointer_y as f32];
        let hit = screen
            .hits
            .iter()
            .rev()
            .find(|(r, _)| contains(*r, point))
            .map(|(_, a)| *a);
        if input.buttons_pressed & buttons::MOUSE_LEFT != 0 {
            screen.pressed = hit;
            let list = if let Some(Action::Thumb(i)) = hit {
                Some(i)
            } else {
                screen
                    .layout
                    .grids
                    .iter()
                    .position(|g| contains(g.clip, point))
            };
            screen.drag = list.map(|i| {
                (
                    i,
                    point[1],
                    screen.offsets[i],
                    hit == Some(Action::Thumb(i)),
                )
            });
        }
        let mut redraw = false;
        if let Some((i, y, offset, thumb)) = screen.drag
            && (input.buttons_down | released) & buttons::MOUSE_LEFT != 0
            && (point[1] - y).abs() > 3.
        {
            let g = screen.layout.grids[i];
            let dy = if thumb {
                (point[1] - y) * (screen.lengths()[i] as f32 * g.step / g.clip[3]).max(1.)
            } else {
                y - point[1]
            };
            screen.offsets[i] = offset + dy;
            screen.pressed = None;
            redraw = true;
        }
        if released & buttons::MOUSE_LEFT != 0 {
            screen.drag = None;
            if let Some(a) = screen.pressed.take().filter(|a| Some(*a) == hit) {
                match a {
                    Action::Event(i) => screen.select(i),
                    Action::Maid(id) => screen.maid = Some(id),
                    Action::Scroll(i, down) => {
                        screen.offsets[i] += if down {
                            screen.layout.grids[i].step
                        } else {
                            -screen.layout.grids[i].step
                        }
                    }
                    Action::Thumb(_) => return Ok(()),
                    Action::Ok | Action::Cancel => {
                        screen.confirm = a == Action::Ok;
                        self.leaving = Some(now);
                        return Ok(());
                    }
                }
                redraw = true;
            }
        }
        if redraw {
            let vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            screen.draw(
                &mut self.widgets,
                &vm.runtime.host().state,
                &mut self.assets,
                op,
                h,
                c,
            )?;
        }
        Ok(())
    }
}

#[test]
#[ignore = "requires the installed standalone 1.0.4 level, prefabs and KAG callers"]
fn installed_event_list_activation_and_return() -> Result<()> {
    let packs = ["sound", "sound2", "parts", "parts2"];
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
    let id = {
        let mut vm = game.script.tjs.lock().unwrap();
        let s = &mut vm.runtime.host_mut().state;
        s.player.schedule_slot.resize_with(40, Default::default);
        crate::tjs::retail::operations::start_day(s)?;
        vm.eval("CreateNewMaid('Muku')")?;
        let s = &mut vm.runtime.host_mut().state;
        crate::tjs::retail::finish_profile(s, true)?;
        let id = s.stock[0];
        s.maids.get_mut(&id).unwrap().likability = 5;
        id
    };
    let tick = |g: &mut Game, rt: &mut crysirror_runtime::Runtime, op: &mut Opener, ms: u64| {
        rt.tick_control(ms, InputFrame::default())?;
        g.update(&mut rt.host, c, op, InputFrame::default())
    };
    let click = |g: &mut Game,
                 rt: &mut crysirror_runtime::Runtime,
                 op: &mut Opener,
                 a: Action|
     -> Result<()> {
        let hits = &g.events.as_ref().unwrap().hits;
        let r = hits
            .iter()
            .find(|(_, act)| *act == a)
            .with_context(|| format!("missing {a:?} in {hits:?}"))?
            .0;
        let input = InputFrame {
            buttons_down: buttons::MOUSE_LEFT,
            buttons_pressed: buttons::MOUSE_LEFT,
            pointer_x: (r[0] + r[2] / 2.) as i32,
            pointer_y: (r[1] + r[3] / 2.) as i32,
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
    for (file, period, confirm) in [
        ("nightmain.ks", daily::Period::Night, false),
        ("daytimemain.ks", daily::Period::Daytime, true),
    ] {
        game.script
            .tjs
            .lock()
            .unwrap()
            .runtime
            .host_mut()
            .state
            .player
            .generic_flag
            .insert("時間帯".into(), if confirm { 2 } else { 3 });
        game.script
            .load(&files, op.storage.as_ref(), file, "*イベント")?;
        game.script.enabled = true;
        game.script.exec();
        game.enter(Scene::Script, &mut op, &mut rt.host, c, 0)?;
        for _ in 0..32 {
            tick(game, &mut rt, &mut op, 250)?;
            if game.scene == Scene::Events {
                break;
            }
        }
        assert_eq!(game.scene, Scene::Events);
        tick(game, &mut rt, &mut op, 500)?;
        if confirm {
            let s = game.events.as_ref().unwrap();
            let n = s
                .list
                .iter()
                .position(|e| e.id == 10)
                .context("missing event 10")?;
            assert!(s.list[n].maids.contains(&id));
            click(game, &mut rt, &mut op, Action::Event(n))?;
            assert_eq!(game.events.as_ref().unwrap().selected, n);
            assert!(game.widgets.iter().any(|w| w.path.ends_with("/Title")
                && w.label.as_ref().is_some_and(|l| !l.text.is_empty())));
        }
        click(
            game,
            &mut rt,
            &mut op,
            if confirm { Action::Ok } else { Action::Cancel },
        )?;
        tick(game, &mut rt, &mut op, 500)?;
        assert_eq!(game.scene, Scene::Script);
        if confirm {
            let mut vm = game.script.tjs.lock().unwrap();
            assert_eq!(vm.eval_string("tf['scenario_file_name']")?, "A1_KAN_0001");
            assert_eq!(vm.eval_string("tf['label_name']")?, "*top");
        } else {
            for _ in 0..32 {
                tick(game, &mut rt, &mut op, 250)?;
                if game.scene == Scene::Daily(period) {
                    break;
                }
            }
            assert_eq!(game.scene, Scene::Daily(period));
        }
    }
    Ok(())
}
