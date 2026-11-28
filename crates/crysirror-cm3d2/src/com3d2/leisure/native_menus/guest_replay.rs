// SPDX-License-Identifier: MPL-2.0
//! The three lists in KasizukiMainMenu.SetUpPlayInfo.
use super::*;

pub(super) const PREFIX: &str = "UI Root/Window Play Info/";
const ROOM: usize = 0;
const MAN: usize = 1;
const PLAY: usize = 2;
const LISTS: [&str; 3] = ["Room List", "Man List", "Play List"];
const GRIDS: [&str; 3] = ["GridPropensitys", "Grid", "Grid"];
const LIST_RECTS: [[f32; 4]; 3] = [
    [109., 184., 366., 749.],
    [486., 184., 228., 749.],
    [723., 184., 460., 749.],
];
const VISIBLE_ROWS: usize = 12;
const ROW_HEIGHT: f32 = 60.;

#[derive(Default)]
pub(super) struct Replay {
    maid: usize,
    default_man: i32,
    rooms: Vec<guest::Room>,
    men: Vec<(i32, String)>,
    plays: Vec<guest::Play>,
    selected: [Option<usize>; 3],
    offsets: [usize; 3],
    hover: Option<usize>,
}

impl Replay {
    fn len(&self, list: usize) -> usize {
        [self.rooms.len(), self.men.len(), self.plays.len()][list]
    }

    fn title(&self, list: usize, row: usize) -> &str {
        match list {
            ROOM => &self.rooms[row].name,
            MAN => &self.men[row].1,
            PLAY => &self.plays[row].name,
            _ => unreachable!(),
        }
    }

    fn scroll(&mut self, list: usize, backwards: bool) {
        let count = self.len(list);
        let offset = &mut self.offsets[list];
        *offset = if backwards {
            offset.saturating_sub(1)
        } else {
            (*offset + 1).min(count.saturating_sub(VISIBLE_ROWS))
        };
    }
}

impl Game {
    pub(super) fn open_guest_replay(&mut self, maid: usize) -> Result<()> {
        let mut vm = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let s = &mut vm.runtime.host_mut().state;
        self.leisure.native_menu.replay = Replay {
            maid,
            default_man: guest::man(s),
            rooms: guest::replay_rooms(s)?,
            ..Default::default()
        };
        drop(vm);
        self.leisure.native_menu.page = Page::GuestReplay;
        self.draw_native_menu(GuestClub)
    }

    pub(super) fn draw_guest_replay(&mut self) -> Result<()> {
        let replay = &self.leisure.native_menu.replay;
        let play = replay
            .hover
            .or(replay.selected[PLAY])
            .and_then(|i| replay.plays.get(i));
        for w in &mut self.widgets {
            if w.path == format!("{PREFIX}OK") && replay.selected.iter().any(Option::is_none) {
                w.collider = None;
                w.tint[3] = w.tint[3].min(100);
            }
            if w.path.ends_with("/Description/Text Description")
                && let Some(label) = &mut w.label
            {
                label.set_text(play.map_or("", |row| &row.description));
            }
        }
        if let Some(play) = play {
            let template = self
                .native_menu_template(&format!("{PREFIX}Description/Parent Condition/temp item"));
            for (i, text) in play.conditions.iter().enumerate() {
                for base in &template {
                    let mut w = base.clone();
                    if let Some(label) = &mut w.label {
                        if w.path.ends_with("/Value") {
                            label.set_text(text);
                        } else if w.path.ends_with("/Number") {
                            label.set_text(&(i + 1).to_string());
                        }
                    }
                    w.collider = None;
                    w.depth += 200;
                    business::move_by(&mut w, 0., i as f32 * 30.);
                    self.widgets.push(w);
                }
            }
        }
        for (list, name) in LISTS.iter().enumerate() {
            let prefix = format!("{PREFIX}{name}/ScrollView/{}/temp item", GRIDS[list]);
            let template = self.native_menu_template(&prefix);
            let offset = replay.offsets[list];
            for i in offset..replay.len(list).min(offset + VISIBLE_ROWS) {
                for base in &template {
                    let mut w = base.clone();
                    if w.path.ends_with("/select sprite") && replay.selected[list] != Some(i) {
                        continue;
                    }
                    if w.path.ends_with("/Value")
                        && let Some(label) = &mut w.label
                    {
                        label.set_text(replay.title(list, i));
                    }
                    if base.path == prefix {
                        w.collider = Some(w.rect);
                    } else {
                        w.collider = None;
                    }
                    business::move_by(&mut w, 0., (i - offset) as f32 * ROW_HEIGHT);
                    w.depth += 200;
                    w.path = format!("guest-replay/{list}/{i}");
                    self.widgets.push(w);
                }
            }
        }
        Ok(())
    }

    pub(super) fn guest_replay_input(&mut self, input: InputFrame) -> Result<()> {
        let replay = &mut self.leisure.native_menu.replay;
        let wheel = input.buttons_pressed & (buttons::WHEEL_UP | buttons::WHEEL_DOWN);
        let scroll = if self.leisure.pressed.is_none() && wheel != 0 {
            LIST_RECTS
                .iter()
                .position(|&rect| business::contains(rect, &input))
        } else {
            None
        };
        if let Some(list) = scroll {
            replay.scroll(list, wheel & buttons::WHEEL_UP != 0);
        }
        let hover = if scroll.is_some() {
            None
        } else {
            self.widgets
                .iter()
                .rev()
                .find(|w| w.collider.is_some_and(|r| business::contains(r, &input)))
                .and_then(|w| w.path.strip_prefix("guest-replay/2/"))
                .and_then(|index| index.parse().ok())
        };
        if scroll.is_some() || hover != replay.hover {
            replay.hover = hover;
            return self.draw_native_menu(GuestClub);
        }
        Ok(())
    }

    pub(super) fn guest_replay_click(
        &mut self,
        path: &str,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<()> {
        let menu = &mut self.leisure.native_menu;
        let replay = &mut menu.replay;
        if let Some((list, index)) = path
            .strip_prefix("guest-replay/")
            .and_then(|row| row.split_once('/'))
        {
            let list = list.parse::<usize>()?;
            let index = index.parse::<usize>()?;
            match list {
                ROOM | MAN => {
                    let mut vm = self
                        .script
                        .tjs
                        .lock()
                        .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                    let s = &mut vm.runtime.host_mut().state;
                    if list == ROOM {
                        let room = replay
                            .rooms
                            .get(index)
                            .context("unknown guest replay room")?
                            .id;
                        let man =
                            replay.selected[MAN].map_or(replay.default_man, |i| replay.men[i].0);
                        replay.men = guest::replay_men(s, room)?;
                        replay.selected[ROOM] = Some(index);
                        replay.selected[MAN] = replay.men.iter().position(|&(id, _)| id == man);
                        replay.offsets[MAN] = 0;
                    } else {
                        ensure!(index < replay.men.len(), "unknown guest replay man");
                        replay.selected[MAN] = Some(index);
                    }
                    replay.plays = match (replay.selected[ROOM], replay.selected[MAN]) {
                        (Some(room), Some(man)) => {
                            guest::plays(s, replay.rooms[room].id, replay.men[man].0, false)?
                        }
                        _ => Vec::new(),
                    };
                    replay.offsets[PLAY] = 0;
                    replay.selected[PLAY] = None;
                    replay.hover = None;
                }
                PLAY if index < replay.plays.len() => replay.selected[PLAY] = Some(index),
                _ => bail!("unknown guest replay row {path}"),
            }
        } else if let Some(control) = path.strip_prefix(PREFIX) {
            match control {
                "Cancel" => {
                    menu.page = Page::GuestFree;
                    menu.replay = Replay::default();
                }
                "OK" => {
                    let [Some(room), Some(man), Some(play)] = replay.selected else {
                        return Ok(());
                    };
                    let (maid, room, man, filename) = (
                        replay.maid,
                        replay.rooms[room].id,
                        replay.men[man].0,
                        replay.plays[play].file.clone(),
                    );
                    return self.launch_guest_scenario(
                        maid,
                        room,
                        man,
                        Some(filename),
                        op,
                        h,
                        c,
                        now,
                    );
                }
                _ => {
                    if let Some((name, arrow)) = control.split_once("/Scroll Bar/")
                        && let Some(list) = LISTS.iter().position(|&list| list == name)
                        && matches!(arrow, "Up" | "Down")
                    {
                        replay.scroll(list, arrow == "Up");
                        replay.hover = None;
                    }
                }
            }
        }
        self.draw_native_menu(GuestClub)
    }
}
