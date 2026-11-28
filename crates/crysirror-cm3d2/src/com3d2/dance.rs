// SPDX-License-Identifier: MPL-2.0
//! Level 21, free dance mode/song/cast selection.
use super::*;
use crate::tjs::{TjsState, retail::activities};
use crysirror_unity::reader::Reader;
pub(super) mod performance;

#[derive(Clone, Default)]
pub(super) struct Layout {
    background: String,
    music: String,
    camera: ([f32; 3], f32, [f32; 2]),
    grids: [shop::Grid; 2],
}
impl Layout {
    pub fn read(&mut self, class: &str, path: &str, pos: [f32; 3], data: &[u8]) -> Result<()> {
        if class == "DanceSelect" {
            let mut r = Reader::new(data);
            r.seek(116)?;
            self.background = r.string()?;
            self.music = r.string()?;
            self.camera = (r.f32s()?, r.f32()?, r.f32s()?);
        }
        if path.contains("/SelectPanel/") {
            self.grids[0].read(class, path, pos, data)?;
        }
        if path.contains("/CharacterSelectPanel/") {
            self.grids[1].read(class, path, pos, data)?;
        }
        Ok(())
    }
}
struct Song {
    title: String,
    size: u16,
    dy: i32,
    count: usize,
    scene: String,
    credits: String,
    image: String,
    live: bool,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    Mode,
    Music,
    Cast,
}
#[derive(Debug, Clone, Copy, PartialEq)]
enum Action {
    Game,
    View,
    Song(usize),
    Maid(usize),
    Scroll(bool),
    Next,
    Back,
}
pub(super) struct Screen {
    base: Vec<Widget>,
    rows: [Vec<Widget>; 2],
    layout: Layout,
    songs: Vec<Song>,
    music: usize,
    cast: Vec<Option<usize>>,
    stage: Stage,
    hits: Vec<([f32; 4], Action)>,
    pressed: Option<Action>,
    ready: u64,
    offset: usize,
    return_key: &'static str,
}
impl Screen {
    pub fn clip(&self, path: &str) -> Option<[f32; 4]> {
        path.starts_with("dance-row/")
            .then(|| self.layout.grids[usize::from(self.stage == Stage::Cast)].clip)
    }
    fn pick(&mut self, index: usize, s: &mut TjsState) {
        self.music = index;
        s.player
            .generic_flag
            .insert("_ライブ背景".into(), i32::from(self.songs[index].live));
    }
    fn draw(
        &mut self,
        widgets: &mut Vec<Widget>,
        s: &TjsState,
        assets: &mut Assets,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
    ) -> Result<()> {
        widgets.clear();
        self.hits.clear();
        let song = self.songs.get(self.music);
        for mut w in self.base.iter().cloned() {
            let p = w.path.as_str();
            if p.contains("/DanceModeSelect/") && self.stage != Stage::Mode
                || p.contains("/MusicSelect/") && self.stage != Stage::Music
                || p.contains("/CharaSelect/") && self.stage != Stage::Cast
            {
                continue;
            }
            if p.ends_with("/ButtonParent/OK") && self.stage == Stage::Mode {
                continue;
            }
            let action = if p.ends_with("/GameMode") {
                Some(Action::Game)
            } else if p.ends_with("/ViewMode") {
                Some(Action::View)
            } else if p.ends_with("/ButtonParent/Cancel") {
                Some(Action::Back)
            } else if p.ends_with("/ButtonParent/OK") {
                if song.is_some()
                    && (self.stage != Stage::Cast || self.cast.iter().any(Option::is_some))
                {
                    Some(Action::Next)
                } else {
                    w.tint = [128, 128, 128, 255];
                    None
                }
            } else if p.ends_with("/Scroll Bar/Up") {
                Some(Action::Scroll(false))
            } else if p.ends_with("/Scroll Bar/Down") {
                Some(Action::Scroll(true))
            } else {
                None
            };
            if let Some(song) = song
                && p.contains("/MujicInfo/BG/")
            {
                if let Some(l) = &mut w.label {
                    if p.ends_with("/TitleFrame/Value") {
                        l.set_text(&song.title.replace('\n', ""));
                    } else if p.ends_with("/InfoFrame/Value") {
                        l.set_text(&song.credits);
                    } else if p.ends_with("/InfoFrame/Value2") {
                        l.set_text(&format!("ダンス人数 : {}人", song.count));
                    }
                } else if p.ends_with("/Image") {
                    let key = assets
                        .unity
                        .resource_of(
                            op,
                            &format!("SceneDanceSelect/Atlas/{}", song.image),
                            crysirror_unity::class_id::TEXTURE_2D,
                        )?
                        .context("missing dance sample image")?;
                    w.asset = assets.texture(op, h, c, key)?.0;
                }
            }
            if let (Some(a), Some(r)) = (action, w.collider) {
                self.hits.push((r, a));
            }
            widgets.push(w);
        }
        if self.stage == Stage::Mode {
            return Ok(());
        }
        let cast = self.stage == Stage::Cast;
        let mut ids = s.stock.clone();
        s.sort_maid_standard(&mut ids);
        let grid = self.layout.grids[cast as usize];
        let n = if cast { ids.len() } else { self.songs.len() };
        let visible = (grid.clip[3] / grid.step).ceil() as usize;
        self.offset = self.offset.min(n.saturating_sub(visible));
        for i in self.offset..n.min(self.offset + visible) {
            for mut w in self.rows[cast as usize].iter().cloned() {
                let a = if cast {
                    let id = ids[i];
                    if !business::prepare_plate(
                        &mut w,
                        id,
                        self.cast.contains(&Some(id)),
                        s,
                        assets,
                        h,
                        c,
                    )? {
                        continue;
                    }
                    Action::Maid(id)
                } else {
                    let song = &self.songs[i];
                    if w.path.to_lowercase().contains("/select") && i != self.music {
                        continue;
                    }
                    if w.path.ends_with("/title")
                        && let Some(l) = &mut w.label
                    {
                        l.set_text(&song.title);
                        l.set_size(song.size);
                        business::move_by(&mut w, 0., -song.dy as f32);
                    }
                    Action::Song(i)
                };
                business::move_by(
                    &mut w,
                    grid.x,
                    grid.clip[1] + ((i - self.offset) as f32 + 0.5) * grid.step - 540.,
                );
                w.depth += 200;
                if let Some(mut r) = w.collider {
                    r[3] = r[3].min(grid.clip[1] + grid.clip[3] - r[1]);
                    self.hits.push((r, a));
                }
                w.path = format!("dance-row/{i}/{}", w.path);
                widgets.push(w);
            }
        }
        Ok(())
    }
}
impl Game {
    pub(super) fn open_dance(
        &mut self,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<String> {
        let mut vm = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
        ensure!(
            vm.eval_bool("CheckDanceType('Free','View')")?,
            "unsupported COM3D2 dance selection mode"
        );
        let s = &mut vm.runtime.host_mut().state;
        let t = s.retail.table("dance_setting.nei")?;
        let mut songs = vec![];
        for r in (1..t.rows).filter(|&i| t.exists(0, i)) {
            if !s.retail.enabled("dance_enabled_list", t.int(0, r))?
                || t.int(10, r) > s.player.flag("_楽曲解放")
            {
                continue;
            }
            let count = usize::try_from(t.int(4, r))?;
            ensure!(count > 0 && count <= 256, "invalid dance cast size {count}");
            songs.push(Song {
                title: t.string(1, r).replace("《改行》", "\n"),
                size: t.int(2, r).try_into()?,
                dy: t.int(3, r),
                count,
                scene: t.string(5, r),
                credits: t.string(6, r).replace("《改行》", "\n"),
                image: t.string(7, r),
                live: t.string(17, r) == "ライブステージ",
            });
        }
        let layout = self.assets.dance_layout.clone();
        ensure!(
            !layout.background.is_empty()
                && layout.grids.iter().all(|g| g.step > 0. && g.clip[3] > 0.),
            "missing dance selection layout"
        );
        let name = layout.background.clone();
        self.adv_camera = layout.camera;
        self.camera_control = true;
        self.camera_move = None;
        self.message_open = false;
        self.dialogue = None;
        self.actors.reset_for_day(s);
        s.active_maids.fill(None);
        self.sound.stop(h, c, Kind::Voice, 0.)?;
        self.sound
            .play(h, c, &self.files, Kind::Bgm, &layout.music, 0.5, true)?;
        let mut screen = Screen {
            base: std::mem::take(&mut self.widgets),
            rows: [
                self.assets.scene(op, h, c, "DanceSongPlate")?,
                self.assets.scene(op, h, c, "PickerPlate")?,
            ],
            layout,
            songs,
            music: 0,
            cast: vec![],
            stage: Stage::Mode,
            hits: vec![],
            pressed: None,
            ready: now + 500,
            offset: 0,
            return_key: "cancel_label",
        };
        if !screen.songs.is_empty() {
            screen.pick(0, s);
        }
        screen.draw(&mut self.widgets, s, &mut self.assets, op, h, c)?;
        self.dance = Some(screen);
        Ok(name)
    }
    pub(super) fn update_dance(
        &mut self,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
        input: InputFrame,
        released: u64,
        now: u64,
    ) -> Result<()> {
        let screen = self.dance.as_mut().context("missing dance selector")?;
        if let Some(start) = self.leaving {
            let key = screen.return_key;
            if now.saturating_sub(start) >= 500 {
                return self.resume_at(key, op, h, c, now);
            }
            return Ok(());
        }
        if now < screen.ready {
            return Ok(());
        }
        let x = input.pointer_x as f32;
        let y = input.pointer_y as f32;
        let hit = screen
            .hits
            .iter()
            .rev()
            .find(|(r, _)| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3])
            .map(|v| v.1);
        if input.buttons_pressed & buttons::MOUSE_LEFT != 0 {
            screen.pressed = hit;
        }
        if released & buttons::MOUSE_LEFT == 0 {
            return Ok(());
        }
        let Some(a) = screen.pressed.take().filter(|v| Some(*v) == hit) else {
            return Ok(());
        };
        let mut vm = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let s = &mut vm.runtime.host_mut().state;
        let before = screen.stage;
        match a {
            Action::Game | Action::View => {
                activities::set_mode(s, if a == Action::View { "View" } else { "Free" })?;
                screen.stage = Stage::Music;
            }
            Action::Scroll(down) => {
                screen.offset = if down {
                    screen.offset + 1
                } else {
                    screen.offset.saturating_sub(1)
                }
            }
            Action::Song(i) => screen.pick(i, s),
            Action::Maid(id) => {
                if screen.cast.len() == 1 {
                    screen.cast[0] = Some(id);
                } else if let Some(i) = screen.cast.iter().position(|v| *v == Some(id)) {
                    screen.cast[i] = None;
                } else if let Some(slot) = screen.cast.iter_mut().find(|v| v.is_none()) {
                    *slot = Some(id);
                }
            }
            Action::Next if screen.stage == Stage::Music => {
                screen.stage = Stage::Cast;
                screen.cast = vec![None; screen.songs[screen.music].count];
                if screen.cast.len() == 1 {
                    let mut ids = s.stock.clone();
                    s.sort_maid_standard(&mut ids);
                    screen.cast[0] = ids.first().copied();
                }
            }
            Action::Next => {
                activities::cast(s, &screen.cast);
                vm.eval(&format!(
                    "tf['dance_scene_name']={}",
                    serde_json::to_string(&screen.songs[screen.music].scene)?
                ))?;
                screen.return_key = "label";
                self.leaving = Some(now);
                return Ok(());
            }
            Action::Back => match screen.stage {
                Stage::Mode => {
                    self.leaving = Some(now);
                    screen.return_key = "cancel_label";
                    return Ok(());
                }
                Stage::Music => screen.stage = Stage::Mode,
                Stage::Cast => screen.stage = Stage::Music,
            },
        }
        if before != screen.stage {
            screen.offset = 0;
            screen.ready = now + 300;
        }
        screen.draw(&mut self.widgets, s, &mut self.assets, op, h, c)
    }
}

#[test]
#[ignore = "requires installed level 21, song plate, enabled music catalog and night script"]
fn installed_free_dance_song_cast_and_cancel() -> Result<()> {
    let mut m = installed_menu::Menu::new()?;
    let hit = |m: &installed_menu::Menu, a| -> Result<[f32; 4]> {
        let hits = &m.game.dance.as_ref().unwrap().hits;
        Ok(hits
            .iter()
            .find(|(_, b)| *b == a)
            .with_context(|| format!("missing {a:?} in {hits:?}"))?
            .0)
    };
    for accept in [false, true] {
        m.open(true, "*ダンス選択", Scene::Dance)?;
        m.click(hit(&m, Action::View)?)?;
        m.tick(300)?;
        let screen = m.game.dance.as_ref().unwrap();
        assert_eq!(screen.stage, Stage::Music);
        ensure!(!screen.songs.is_empty(), "empty free-dance catalog");
        m.click(hit(&m, Action::Song(0))?)?;
        let scene = m.game.dance.as_ref().unwrap().songs[0].scene.clone();
        m.click(hit(&m, Action::Next)?)?;
        m.tick(300)?;
        assert_eq!(m.game.dance.as_ref().unwrap().stage, Stage::Cast);
        if accept {
            let id = m.game.script.tjs.lock().unwrap().runtime.host().state.stock[0];
            m.click(hit(&m, Action::Maid(id))?)?;
            m.click(hit(&m, Action::Next)?)?;
            m.tick(500)?;
            let mut t = m.game.script.tjs.lock().unwrap();
            assert_eq!(t.eval_string("tf['dance_scene_name']")?, scene);
            assert_eq!(t.runtime.host().state.active_maids[0], Some(id));
            assert_eq!(
                t.runtime.host().state.tmp_flags.get("ダンス設定人数"),
                Some(&1)
            );
        } else {
            for _ in 0..3 {
                m.click(hit(&m, Action::Back)?)?;
                m.tick(300)?;
            }
            m.wait(Scene::Daily(daily::Period::Night))?;
        }
    }
    Ok(())
}
