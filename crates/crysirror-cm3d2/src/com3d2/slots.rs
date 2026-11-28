// SPDX-License-Identifier: MPL-2.0
//! GameMain and SaveAndLoadMgr's standalone v1040 slot headers.
use super::*;
use crate::{
    binio::Writer,
    save::{Header, save_path, slot_1040::{Guest, edit_guest_slots, read_guest_slots}},
    tjs::retail::guest,
};
use crysirror_core::{
    path::Mount,
    service::{ClientStorage, TextInputSessionHandle, TextInputSessionSpec},
};
use std::{
    collections::BTreeMap,
    io::{BufReader, Read, Seek},
};

fn scalar<const N: usize>(r: &mut impl Read) -> Result<[u8; N]> {
    let mut b = [0; N];
    r.read_exact(&mut b)?;
    Ok(b)
}

// BinaryReader.ReadString. Reading incrementally keeps listing a slot independent
// of the size of its character, scenario and facility sections.
fn string(r: &mut impl Read) -> Result<String> {
    let mut len = 0u32;
    for shift in (0..35).step_by(7) {
        let [b] = scalar(r)?;
        ensure!(shift != 28 || b <= 7, "invalid BinaryWriter string length");
        len |= u32::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            let mut bytes = Vec::new();
            let count = r.take(len.into()).read_to_end(&mut bytes)?;
            ensure!(count == len as usize, "truncated save header string");
            return Ok(String::from_utf8(bytes)?);
        }
    }
    bail!("invalid save header string length")
}

fn read_header(r: &mut impl Read) -> Result<Header> {
    ensure!(
        string(r)? == "COM3D2_SAVE",
        "invalid standalone save header"
    );
    let version = i32::from_le_bytes(scalar(r)?);
    ensure!(
        version == 1040,
        "unsupported standalone save version {version}"
    );
    Ok(Header {
        save_time: string(r)?,
        game_day: i32::from_le_bytes(scalar(r)?),
        player_name: string(r)?,
        maid_num: i32::from_le_bytes(scalar(r)?),
        comment: string(r)?,
    })
}

fn catalog(storage: &dyn ClientStorage) -> Result<BTreeMap<i32, Header>> {
    let mut out = BTreeMap::new();
    if storage.metadata(Mount::Save, "SaveData")?.is_none() {
        return Ok(out);
    }
    for name in storage.list(Mount::Save, "SaveData")? {
        let Some(digits) = name
            .strip_prefix("SaveData")
            .and_then(|s| s.strip_suffix(".save"))
        else {
            continue;
        };
        if digits.len() != 3 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let slot = digits.parse::<i32>()?;
        if slot >= 100 {
            continue;
        }
        let reader = storage.open_read(&save_path(slot)?)?;
        out.insert(
            slot,
            read_header(&mut BufReader::new(crysirror_resource::AssetIo::new(
                reader,
            )?))
            .with_context(|| format!("standalone save slot {slot}"))?,
        );
    }
    Ok(out)
}

// Both BinaryBookmark offsets are relative to the end of their own Int64.
// Replacing only the header leaves the complete body and both offsets intact.
fn change_comment(
    storage: &dyn ClientStorage,
    slot: i32,
    expected: &Header,
    value: &str,
) -> Result<Header> {
    let path = save_path(slot)?;
    let bytes = storage.read(&path)?;
    let mut input = std::io::Cursor::new(&bytes);
    let mut header = read_header(&mut input)?;
    ensure!(
        &header == expected,
        "the selected save slot changed while editing"
    );
    let body = &bytes[usize::try_from(input.stream_position()?)?..];
    let mut b = crate::binio::Reader::new(body);
    crate::save::player_1040::bookmarks(&mut b, body.len())?;
    let units = value.encode_utf16().take(25).collect::<Vec<_>>();
    header.comment = String::from_utf16_lossy(&units);
    let mut writer = Writer::default();
    writer.str("COM3D2_SAVE");
    writer.i32(1040);
    crate::save::write_header(&mut writer, &header);
    writer.bytes.try_reserve(body.len())?;
    writer.bytes.extend_from_slice(body);
    storage.write_atomic(&path, &writer.bytes)?;
    Ok(header)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Switch(Mode),
    Page(i32),
    Slot(i32),
    Comment(i32),
    Cancel,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Mode {
    TitleLoad,
    TitleGuest,
    GuestTitleLoad,
    DailyLoad,
    DailySave,
    GuestLoad,
    GuestSave,
}

impl Mode {
    fn local(self) -> bool {
        matches!(self, Self::GuestTitleLoad | Self::GuestLoad | Self::GuestSave)
    }
    fn saving(self) -> bool {
        matches!(self, Self::DailySave | Self::GuestSave)
    }
    fn rows(self) -> i32 {
        if self.local() { 16 } else { 10 }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Confirmation {
    Load,
    Save,
    Delete,
    NewGuest,
    Notice,
}

#[derive(Debug, PartialEq, Eq)]
enum SlotOperation {
    Save { slot: i32, comment: String },
    Load { slot: i32 },
    DeleteGuest { slot: i32 },
    NewGuest,
}

pub(super) struct Viewer {
    entries: BTreeMap<i32, Header>,
    base: Vec<Widget>,
    full: Vec<Widget>,
    empty: Vec<Widget>,
    latest: Option<i32>,
    page: i32,
    active: Option<i32>,
    pressed: Option<Action>,
    editing: Option<(i32, TextInputSessionHandle, bool)>,
    pub dialog: Option<quit_dialog::QuitDialog>,
    confirmation: Confirmation,
    mode: Mode,
    caller_widgets: Vec<Widget>,
    right_pressed: Option<i32>,
    opened: u64,
    closing: Option<u64>,
    operation: Option<SlotOperation>,
    origin: Option<i32>,
    guest_layout: Option<TableLayout>,
}

fn latest(entries: &BTreeMap<i32, Header>) -> Option<i32> {
    let mut latest = None;
    let mut newest = -1i64;
    for (&slot, h) in entries {
        if let Ok(stamp) = h.save_time.parse::<i64>()
            && stamp > newest
        {
            newest = stamp;
            latest = Some(slot);
        }
    }
    latest
}

impl Viewer {
    pub fn open(
        mode: Mode,
        assets: &mut Assets,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<Self> {
        let entries = catalog(op.storage.as_ref())?;
        Self::open_with(mode, assets, op, host, c, now, entries, None)
    }

    fn open_with(
        mode: Mode,
        assets: &mut Assets,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
        now: u64,
        entries: BTreeMap<i32, Header>,
        origin: Option<i32>,
    ) -> Result<Self> {
        let latest = latest(&entries);
        let mut base = assets.scene(op, host, c, match mode {
            Mode::TitleLoad | Mode::TitleGuest => "StandaloneLoadPanel",
            local if local.local() => "GuestSlotPanel",
            _ => "DailySlotPanel",
        })?;
        let (full, empty, guest_layout) = if mode.local() {
            let mut unit = Vec::new();
            base.retain(|w| {
                if let Some(path) = w.path.strip_prefix("UI Root/SaveAndLoadPanel/DataViewer/DataUnitParent/") {
                    let mut row = w.clone();
                    row.path = path.into();
                    unit.push(row);
                    false
                } else {
                    true
                }
            });
            ensure!(unit.iter().any(|w| w.path == "DataUnit" && w.collider.is_some()), "missing native guest slot row");
            (unit.clone(), unit, Some(assets.guest_slot_table))
        } else {
            (assets.scene(op, host, c, "StandaloneDataUnit")?,
             assets.scene(op, host, c, "StandaloneEmptyUnit")?, None)
        };
        Ok(Self {
            entries,
            base, full, empty, guest_layout, origin,
            latest,
            page: latest.map_or(1, |i| i / mode.rows() + 1),
            active: None,
            pressed: None,
            editing: None,
            dialog: None,
            mode,
            caller_widgets: Vec::new(),
            right_pressed: None,
            confirmation: Confirmation::Notice,
            opened: now,
            closing: None,
            operation: None,
        })
    }
    pub fn alpha(&self, now: u64) -> u8 {
        let alpha = if let Some(t) = self.closing {
            1. - (now.saturating_sub(t) as f32 / 500.)
        } else {
            now.saturating_sub(self.opened) as f32 / 500.
        };
        (alpha.clamp(0., 1.) * 255.).round() as u8
    }
    fn populate(&self, layout: TableLayout) -> Result<Vec<Widget>> {
        let mut out = Vec::new();
        let saving = self.mode.saving();
        for w in self.base.iter().cloned() {
            if w.path.contains("/TitleGroup/TitleSave") && !saving
                || w.path.contains("/TitleGroup/TitleLoad") && saving
                || w.path.contains("/DisplayButtonGroup/DisplaySave") && saving
                || w.path.contains("/DisplayButtonGroup/DisplaySave") && self.mode == Mode::GuestTitleLoad
                || w.path.contains("/DisplayButtonGroup/DisplayLoad") && !saving
            {
                continue;
            }
            if w.path.ends_with("/SelectCursor")
                && !w.path.contains(&format!("/Page_{}/", self.page))
            {
                continue;
            }
            out.push(w);
        }
        let mut rows = Vec::new();
        for n in 0..self.mode.rows() {
            let slot = (self.page - 1) * self.mode.rows() + n;
            let h = self.entries.get(&slot);
            let mut unit = if h.is_some() {
                self.full.clone()
            } else {
                self.empty.clone()
            };
            unit.retain(|w| {
                (!w.path.ends_with("/NewLabel") || self.latest == Some(slot))
                    && (!w.path.ends_with("/SelectCursor") || self.active == Some(slot))
            });
            for w in &mut unit {
                if let (Some(l), Some(h)) = (&mut w.label, h) {
                    let text = if w.path.ends_with("/DataInfo/Date") {
                        Some(crate::saveload::date_text(&h.save_time))
                    } else if w.path.ends_with("/LapsedDays/Number") {
                        Some(h.game_day.to_string())
                    } else if w.path.ends_with("/UsedCount/Number") {
                        Some(h.game_day.to_string())
                    } else if w.path.ends_with("/Manager/Name") || w.path.ends_with("/Man/Name") {
                        Some(h.player_name.clone())
                    } else if w.path.ends_with("/NumberOfEmployees/Number") {
                        Some(h.maid_num.to_string())
                    } else if w.path.ends_with("/InputField/Label") {
                        Some(h.comment.clone())
                    } else {
                        None
                    };
                    if let Some(text) = text {
                        l.set_text(&text);
                    }
                }
                w.path = format!("serialNo_{slot}/{}", w.path);
            }
            rows.push(unit);
        }
        self.guest_layout.unwrap_or(layout).position(&mut rows)?;
        if self.mode.local() {
            for (n, unit) in rows.iter_mut().enumerate() {
                let slot = (self.page - 1) * 16 + n as i32;
                let occupied = self.entries.contains_key(&slot);
                unit.retain(|w| if w.path.ends_with("/NewGameLabel") {
                    n == 0
                } else if w.path.contains("/DataUnit/Content/") {
                    occupied
                } else {
                    true
                });
            }
        }
        out.extend(rows.into_iter().flatten());
        out.sort_by_key(|w| w.depth);
        Ok(out)
    }
    fn hit(&self, widgets: &[Widget], input: &InputFrame) -> Option<Action> {
        // The input field overlays the slot's broader collider.
        let mut hit = None;
        for w in widgets {
            let Some(r) = w.collider else { continue };
            if !business::contains(r, input) {
                continue;
            }
            if let Some(tail) = w.path.strip_prefix("serialNo_") {
                let (num, _) = tail.split_once('/')?;
                let slot = num.parse().ok()?;
                if !self.entries.contains_key(&slot) {
                    if self.mode.local() && w.path.ends_with("/DataUnit")
                        && (self.mode.saving() != (slot % 16 == 0))
                    {
                        hit = Some(Action::Slot(slot));
                    } else if self.mode == Mode::DailySave && w.path.ends_with("/EmptyUnit") {
                        hit = Some(Action::Slot(slot));
                    }
                    continue;
                }
                if w.path.ends_with("/InputField") {
                    return Some(Action::Comment(slot));
                }
                if w.path.ends_with("/DataUnit")
                    && !(self.mode == Mode::GuestSave && slot % 16 == 0)
                {
                    hit = Some(Action::Slot(slot));
                }
            } else if w.path.ends_with("/DisplaySave") {
                return Some(Action::Switch(if self.mode.local() { Mode::GuestSave } else { Mode::DailySave }));
            } else if w.path.ends_with("/DisplayLoad") {
                return Some(Action::Switch(if self.mode.local() { Mode::GuestLoad } else { Mode::DailyLoad }));
            } else if w.path.ends_with("/Cancel") {
                return Some(Action::Cancel);
            } else if let Some(page) = w
                .path
                .rsplit('/')
                .next()
                .and_then(|v| v.strip_prefix("Page_"))
                .and_then(|v| v.parse().ok())
            {
                return Some(Action::Page(page));
            }
        }
        hit
    }

    fn blur(&mut self, host: &mut dyn HostServices, c: ClientId) -> Result<()> {
        if let Some((_, h, _)) = self.editing.take() {
            host.release_text_input_session(c, h)?;
        }
        Ok(())
    }

    pub fn input(
        &mut self,
        widgets: &mut Vec<Widget>,
        assets: &mut Assets,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
        input: InputFrame,
        released: u64,
        now: u64,
    ) -> Result<bool> {
        if let Some(t) = self.closing {
            return Ok(now.saturating_sub(t) >= 500);
        }
        if let Some(d) = &mut self.dialog {
            if let Some(a) = d.update(widgets, input, released, now) {
                widgets.truncate(d.first);
                self.dialog = None;
                let action = self.confirmation;
                self.confirmation = Confirmation::Notice;
                if a == quit_dialog::Choice::Ok && action == Confirmation::NewGuest {
                    self.operation = Some(SlotOperation::NewGuest);
                } else if a == quit_dialog::Choice::Ok
                    && matches!(action, Confirmation::Load | Confirmation::Save)
                {
                    let slot = self.active.context("missing confirmed slot")?;
                    self.operation = Some(if action == Confirmation::Save {
                        let comment = self.entries.get(&slot).map(|header| header.comment.clone()).unwrap_or_default();
                        SlotOperation::Save { slot, comment }
                    } else {
                        SlotOperation::Load { slot }
                    });
                } else {
                    if a == quit_dialog::Choice::Ok && action == Confirmation::Delete {
                        let slot = self.active.context("missing confirmed slot")?;
                        if self.mode.local() {
                            self.operation = Some(SlotOperation::DeleteGuest { slot });
                            return Ok(false);
                        }
                        op.storage.remove(&save_path(slot)?)?;
                        self.entries = catalog(op.storage.as_ref())?;
                        // PageUpper rebuilds this page. GameMain keeps the latest-slot number until UpdateSaveDataDay.
                    }
                    self.active = None;
                    *widgets = self.populate(assets.slot_table)?;
                }
            }
            return Ok(false);
        }
        if let Some((slot, h, selected)) = self.editing {
            let added = host.take_text_input_committed(c, h)?;
            let back = input.buttons_pressed & buttons::BACKSPACE != 0;
            if back || !added.is_empty() {
                let old = self
                    .entries
                    .get(&slot)
                    .context("missing edited save slot")?;
                let mut text = if selected {
                    String::new()
                } else {
                    old.comment.clone()
                };
                if back && !selected {
                    text.pop();
                }
                text.push_str(&added);
                let updated = if self.mode.local() {
                    let mut header = old.clone();
                    let limit = assets.guest_slot_comment_limit;
                    if limit != 0 {
                        text = String::from_utf16_lossy(&text.encode_utf16().take(limit).collect::<Vec<_>>());
                    }
                    header.comment.clone_from(&text);
                    header
                } else {
                    change_comment(op.storage.as_ref(), slot, old, &text)?
                };
                self.entries.insert(slot, updated);
                self.editing = Some((slot, h, false));
                *widgets = self.populate(assets.slot_table)?;
            }
            if input.buttons_pressed & (buttons::ENTER | buttons::ESCAPE) != 0 {
                self.blur(host, c)?;
            }
        }
        let hit = self.hit(widgets, &input);
        let occupied = match hit {
            Some(Action::Slot(slot)) if self.entries.contains_key(&slot) => Some(slot),
            _ => None,
        };
        if input.buttons_pressed & buttons::MOUSE_RIGHT != 0 {
            self.right_pressed = occupied;
        }
        if released & buttons::MOUSE_RIGHT != 0 {
            let pressed = self.right_pressed.take();
            if let Some(slot) = occupied
                && pressed == Some(slot)
            {
                self.blur(host, c)?;
                self.active = Some(slot);
                self.confirmation = Confirmation::Delete;
                *widgets = self.populate(assets.slot_table)?;
                self.dialog = Some(quit_dialog::QuitDialog::show(
                    widgets,
                    assets.scene(op, host, c, "SystemDialog")?,
                    now,
                    "データを削除しますが宜しいですか。",
                )?);
            }
            return Ok(false);
        }
        if input.buttons_pressed & buttons::MOUSE_LEFT != 0 {
            self.pressed = hit;
        }
        if released & buttons::MOUSE_LEFT == 0 || self.pressed.take() != hit {
            return Ok(false);
        }
        if !matches!(hit,Some(Action::Comment(slot)) if self.editing.is_some_and(|v|v.0==slot)) {
            self.blur(host, c)?;
        }
        match hit {
            Some(Action::Cancel) => {
                self.closing = Some(now);
            }
            Some(Action::Switch(mode)) => {
                self.mode = mode;
                self.page = self.latest.map_or(1, |slot| slot / mode.rows() + 1);
                self.active = None;
                *widgets = self.populate(assets.slot_table)?;
            }
            Some(Action::Page(p)) => {
                self.page = p;
                self.active = None;
                *widgets = self.populate(assets.slot_table)?;
            }
            Some(Action::Slot(slot)) => {
                self.active = Some(slot);
                *widgets = self.populate(assets.slot_table)?;
                let message = if self.mode.local() {
                    if slot % 16 == 0 {
                        self.confirmation = Confirmation::NewGuest;
                        "ここに新規顧客情報を登録します。".to_owned()
                    } else if self.mode.saving() {
                        self.confirmation = Confirmation::Save;
                        if self.entries.contains_key(&slot) {
                            format!("{slot}番に上書きして保存します。\n宜しいですか？")
                        } else {
                            format!("{slot}番にデータを保存しますか？")
                        }
                    } else {
                        self.confirmation = Confirmation::Load;
                        format!("{slot}番のデータをロードしますか？")
                    }
                } else if self.mode.saving() {
                    self.confirmation = Confirmation::Save;
                    if self.entries.contains_key(&slot) {
                        "上書きして保存しますが宜しいですか？".to_owned()
                    } else {
                        "データを保存しますか？".to_owned()
                    }
                } else {
                    self.confirmation = Confirmation::Load;
                    "データをロードしますか？".to_owned()
                };
                self.dialog = Some(quit_dialog::QuitDialog::show(
                    widgets,
                    assets.scene(op, host, c, "SystemDialog")?,
                    now,
                    &message,
                )?);
            }
            Some(Action::Comment(slot)) if self.editing.is_none() => {
                self.active = Some(slot);
                let r = widgets
                    .iter()
                    .find(|w| {
                        w.path.starts_with(&format!("serialNo_{slot}/"))
                            && w.path.ends_with("/InputField")
                    })
                    .context("missing comment input field")?
                    .rect;
                let h = host.open_text_input_session(
                    c,
                    TextInputSessionSpec {
                        capture: true,
                        cursor_area: Some(r.map(|n| n as i32)),
                        ..Default::default()
                    },
                )?;
                self.editing = Some((slot, h, true));
                *widgets = self.populate(assets.slot_table)?;
            }
            _ => {}
        }
        Ok(false)
    }
}

// NGUI UITable.RepositionVariableSize, downward and upper-left aligned in SceneTitle.
#[derive(Clone, Copy, Default)]
pub(super) struct TableLayout {
    origin: [f32; 2],
    padding: [f32; 2],
    cols: usize,
}
impl TableLayout {
    pub fn read(&mut self, offset: [f32; 3], bytes: &[u8]) -> Result<()> {
        let mut r = crysirror_unity::reader::Reader::new(bytes);
        r.seek(32)?;
        self.cols = r.i32()?.try_into()?;
        ensure!(
            self.cols > 0 && r.i32()? == 0 && r.i32()? == 0 && r.i32()? == 0 && r.i32()? == 0,
            "unknown save UITable arrangement"
        );
        r.seek(60)?;
        self.padding = r.f32s::<2>()?;
        self.origin = [960. + offset[0], 540. - offset[1]];
        Ok(())
    }
    fn position(self, rows: &mut [Vec<Widget>]) -> Result<()> {
        ensure!(self.cols > 0, "missing save table");
        let dim = |unit: &[Widget]| -> Result<[f32; 4]> {
            let mut low = [f32::INFINITY; 2];
            let mut high = [f32::NEG_INFINITY; 2];
            for w in unit {
                for i in 0..2 {
                    low[i] = low[i].min(w.rect[i]);
                    high[i] = high[i].max(w.rect[i] + w.rect[i + 2]);
                }
            }
            ensure!(low.iter().all(|v| v.is_finite()), "empty save prefab");
            Ok([low[0], low[1], high[0] - low[0], high[1] - low[1]])
        };
        let bounds = rows.iter().map(|v| dim(v)).collect::<Result<Vec<_>>>()?;
        let mut widths = vec![0f32; self.cols];
        let mut heights = vec![0f32; rows.len().div_ceil(self.cols)];
        for (i, r) in bounds.iter().enumerate() {
            widths[i % self.cols] = widths[i % self.cols].max(r[2]);
            heights[i / self.cols] = heights[i / self.cols].max(r[3]);
        }
        for (i, unit) in rows.iter_mut().enumerate() {
            let (col, row) = (i % self.cols, i / self.cols);
            let dx = self.origin[0]
                + widths[..col].iter().sum::<f32>()
                + (2 * col + 1) as f32 * self.padding[0]
                - bounds[i][0];
            let dy = self.origin[1]
                + heights[..row].iter().sum::<f32>()
                + (2 * row + 1) as f32 * self.padding[1]
                - bounds[i][1];
            for w in unit {
                business::move_by(w, dx, dy);
            }
        }
        Ok(())
    }
}

impl Game {
    fn guest_slot_headers(&self, slots: &[Guest], maid_count: i32) -> Result<BTreeMap<i32, Header>> {
        // The shared slot viewer uses the same header projection for its
        // two panels. Guest rows display play count and customer name.
        let mut script = self.script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let people = guest::men(&mut script.runtime.host_mut().state)?;
        Ok(slots.iter().filter(|slot| (0..80).contains(&slot.slot)).map(|slot| {
            let man = people.iter().find(|(id, _)| *id == slot.last_man_type)
                .map(|(_, name)| name.clone()).unwrap_or_else(|| slot.last_man_type.to_string());
            (slot.slot, Header {
                save_time: slot.save_time.clone(),
                game_day: slot.play_count,
                player_name: man,
                maid_num: maid_count,
                comment: slot.comment.clone(),
            })
        }).collect())
    }

    pub(super) fn open_guest_slots(
        &mut self, mode: Mode, op: &mut Opener, host: &mut dyn HostServices, c: ClientId, now: u64,
    ) -> Result<()> {
        let (origin, slots, maid_count) = {
            let script = self.script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let state = &script.runtime.host().state;
            (guest::origin(state)?, guest::saved_slots(state).to_vec(),
                i32::try_from(guest::eligible_maids(state).len())?)
        };
        self.replace_guest_panel(mode, origin, &slots, maid_count, op, host, c, now)
    }

    fn replace_guest_panel(
        &mut self, mode: Mode, origin: i32, slots: &[Guest], maid_count: i32,
        op: &mut Opener, host: &mut dyn HostServices, c: ClientId, now: u64,
    ) -> Result<()> {
        let entries = self.guest_slot_headers(slots, maid_count)?;
        let mut viewer = Viewer::open_with(mode, &mut self.assets, op, host, c, now, entries, Some(origin))?;
        let panel = viewer.populate(self.assets.slot_table)?;
        viewer.caller_widgets = if let Some(old) = self.slot_viewer.take() {
            old.caller_widgets
        } else {
            std::mem::take(&mut self.widgets)
        };
        self.widgets = panel;
        self.slot_viewer = Some(viewer);
        self.pressed = None;
        Ok(())
    }

    fn load_guest_in_club(
        &mut self, origin: i32, slot: Option<i32>, op: &mut Opener, now: u64,
    ) -> Result<()> {
        let (old, id) = {
            let mut script = self.script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let state = &mut script.runtime.host_mut().state;
            let mut staged = state.clone();
            let saved = slot.map(|slot| guest::saved_slots(state).iter()
                .find(|record| record.slot == slot).context("guest slot missing")).transpose()?;
            guest::apply_slot(&mut staged, origin, saved)?;
            let id = guest::man(&staged);
            (std::mem::replace(state, staged), id)
        };
        if let Err(err) = self.show_guest_man(id, op, now) {
            self.script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?.runtime.host_mut().state = old;
            return Err(err);
        }
        self.close_slot_viewer()?;
        Ok(())
    }

    fn close_slot_viewer(&mut self) -> Result<()> {
        let viewer = self.slot_viewer.take().context("slot panel closed")?;
        if matches!(viewer.mode, Mode::GuestLoad | Mode::GuestSave) {
            self.copy_guest_comments(&viewer.entries)?;
        }
        self.widgets = viewer.caller_widgets;
        self.pressed = None;
        Ok(())
    }

    fn copy_guest_comments(&mut self, entries: &BTreeMap<i32, Header>) -> Result<()> {
        let mut vm = self.script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
        apply_guest_comments(entries, guest::saved_slots_mut(&mut vm.runtime.host_mut().state));
        Ok(())
    }

    fn write_guest_operation(
        &mut self, origin: i32, operation: SlotOperation, op: &Opener, mode: Mode,
    ) -> Result<()> {
        let viewer = self.slot_viewer.as_ref().context("missing guest slot panel")?;
        let maid_count = if mode == Mode::GuestTitleLoad {
            catalog(op.storage.as_ref())?.get(&origin).context("campaign slot disappeared")?.maid_num
        } else {
            let script = self.script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
            i32::try_from(guest::eligible_maids(&script.runtime.host().state).len())?
        };
        let path = save_path(origin)?;
        let bytes = op.storage.read(&path)?;
        let revised = edit_guest_slots(&bytes, |slots| {
            apply_guest_comments(&viewer.entries, slots);
            match operation {
                SlotOperation::Save { slot, comment } => {
                    let script = self.script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
                    let data = guest::capture_slot(&script.runtime.host().state, slot, comment);
                    if let Some(old) = slots.iter_mut().find(|old| old.slot == slot) {
                        *old = data;
                    } else {
                        slots.push(data);
                    }
                }
                SlotOperation::DeleteGuest { slot } => slots.retain(|saved| saved.slot != slot),
                _ => bail!("wrong guest slot operation"),
            }
            Ok(())
        })?;
        let slots = read_guest_slots(&revised)?;
        let entries = self.guest_slot_headers(&slots, maid_count)?;
        op.storage.write_atomic(&path, &revised)?;
        if mode != Mode::GuestTitleLoad {
            let mut script = self.script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
            guest::set_saved_slots(&mut script.runtime.host_mut().state, slots);
        }
        let viewer = self.slot_viewer.as_mut().context("missing guest slot panel")?;
        viewer.entries = entries;
        viewer.latest = latest(&viewer.entries);
        self.widgets = viewer.populate(self.assets.slot_table)?;
        Ok(())
    }

    pub(super) fn open_slots(
        &mut self,
        mode: Mode,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<()> {
        let mut viewer = Viewer::open(mode, &mut self.assets, op, host, c, now)?;
        let panel = viewer.populate(self.assets.slot_table)?;
        viewer.caller_widgets = std::mem::replace(&mut self.widgets, panel);
        self.slot_viewer = Some(viewer);
        self.pressed = None;
        Ok(())
    }
    pub(super) fn update_slots(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
        input: InputFrame,
        released: u64,
        now: u64,
    ) -> Result<()> {
        let viewer = self.slot_viewer.as_mut().context("slot panel is closed")?;
        if viewer.input(
            &mut self.widgets,
            &mut self.assets,
            op,
            host,
            c,
            input,
            released,
            now,
        )? {
            let mode = viewer.mode;
            self.close_slot_viewer()?;
            if mode == Mode::GuestTitleLoad {
                self.open_slots(Mode::TitleGuest, op, host, c, now)?;
            }
            return Ok(());
        }
        let Some(operation) = viewer.operation.take() else {
            return Ok(());
        };
        let was_save = matches!(&operation, SlotOperation::Save { .. });
        let mode = viewer.mode;
        if mode == Mode::TitleGuest && let SlotOperation::Load { slot } = operation {
            let result = (|| {
                let bytes = op.storage.read(&save_path(slot)?)?;
                let saved = crate::save::slot_1040::Slot::read(&bytes)?;
                let original_staff = saved.roster.employees.iter().filter(|employee| {
                    matches!(employee.status.heroine, crate::save::status_1040::Heroine::Original)
                }).count();
                self.replace_guest_panel(Mode::GuestTitleLoad, slot, &saved.guest_slots,
                    i32::try_from(original_staff)?, op, host, c, now)
            })();
            return match result {
                Ok(()) => Ok(()),
                Err(err) => self.slot_notice(op, host, c, now,
                    &format!("セーブデータの読み込みに失敗しました。\n{err:#}"), false),
            };
        }
        if mode.local() {
            let origin = viewer.origin.context("missing campaign origin")?;
            return match operation {
                SlotOperation::Save { .. } | SlotOperation::DeleteGuest { .. } => {
                    let text = match self.write_guest_operation(origin, operation, op, mode) {
                        Ok(()) if was_save => "データを保存しました。".to_owned(),
                        Ok(()) => "データを削除しました。".to_owned(),
                        Err(err) => format!("セーブデータの書き込みに失敗しました。\n{err:#}"),
                    };
                    self.slot_notice(op, host, c, now, &text, true)
                }
                load => {
                    let selection = match load {
                        SlotOperation::NewGuest => None,
                        SlotOperation::Load { slot } => Some(slot),
                        _ => unreachable!(),
                    };
                    let result = if mode == Mode::GuestTitleLoad {
                        let entries = viewer.entries.clone();
                        self.load_native_guest(op, host, c, origin, selection, now)
                            .and_then(|()| self.copy_guest_comments(&entries))
                    } else {
                        self.load_guest_in_club(origin, selection, op, now)
                    };
                    match result {
                        Ok(()) => Ok(()),
                        Err(err) => self.slot_notice(op, host, c, now,
                            &format!("セーブデータの読み込みに失敗しました。\n{err:#}"), false),
                    }
                }
            };
        }
        let message = match operation {
            SlotOperation::Save { slot, comment } => {
                match self.write_native_slot(op, slot, comment) {
                    Ok(()) => {
                        let viewer = self.slot_viewer.as_mut().context("slot viewer closed during save")?;
                        viewer.entries = catalog(op.storage.as_ref())?;
                        self.widgets = viewer.populate(self.assets.slot_table)?;
                        "データが保存されました。".to_owned()
                    }
                    Err(err) => format!("セーブデータの保存に失敗しました。\n{err:#}"),
                }
            }
            SlotOperation::Load { slot } => {
                match self.load_native_slot(op, host, c, slot, now) {
                    Ok(()) => return Ok(()),
                    Err(err) => format!("セーブデータの読み込みに失敗しました。\n{err:#}"),
                }
            }
            _ => bail!("guest-only action in campaign slot panel"),
        };
        self.slot_notice(op, host, c, now, &message, was_save)
    }

    fn slot_notice(
        &mut self, op: &mut Opener, host: &mut dyn HostServices, c: ClientId, now: u64,
        message: &str, clear_selection: bool,
    ) -> Result<()> {
        let viewer = self.slot_viewer.as_mut().context("slot panel closed during operation")?;
        let panel = self.assets.scene(op, host, c, "SystemDialog")?;
        viewer.dialog = Some(quit_dialog::QuitDialog::show(&mut self.widgets, panel, now, message)?);
        self.widgets.retain(|w| !w.path.contains("SystemDialog/Base/Cancel"));
        viewer.confirmation = Confirmation::Notice;
        if clear_selection {
            viewer.active = None;
        }
        Ok(())
    }
}

fn apply_guest_comments(entries: &BTreeMap<i32, Header>, records: &mut [Guest]) {
    for record in records {
        if let Some(header) = entries.get(&record.slot)
            && header.save_time == record.save_time
        {
            record.comment.clone_from(&header.comment);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn slot(time: &str, comment: &str) -> (Vec<u8>, Vec<u8>) {
        let mut stock = Writer::default();
        stock.i32(0);
        stock.i32(0);
        stock.str("CM3D2_SCRIPT");
        stock.i32(1040);
        slot_with_stock(time, comment, &stock.bytes)
    }

    fn slot_with_stock(time: &str, comment: &str, stock: &[u8]) -> (Vec<u8>, Vec<u8>) {
        slot_with_sections(time, comment, stock, &[33, 44, 55], &[77, 88])
    }

    fn slot_with_sections(
        time: &str,
        comment: &str,
        stock: &[u8],
        guest: &[u8],
        casino: &[u8],
    ) -> (Vec<u8>, Vec<u8>) {
        // GameMain.Serialize writes the two bookmark tags and patches each
        // distance after its managed section. The section contents are opaque
        // to GetSaveDataHeader and SaveDataCommentSave.
        let mut body = Writer::default();
        body.str("bookmark_kk_01");
        let first = body.bytes.len();
        body.i64(-1);
        body.str("bookmark_kk_02");
        let second = body.bytes.len();
        body.i64(-1);
        body.bool(false);
        body.i32(1);
        body.str("base");
        body.i32(0);
        body.str("CM3D2_CHR_MGR");
        body.i32(1040);
        let p = crate::save::player_1040::Player {
            days: 12,
            name: "支配人".into(),
            club_name: "エンパイアクラブ".into(),
            money: 1_234_567,
            ..Default::default()
        };
        body.bytes.extend(p.encode().unwrap());
        body.bytes.extend_from_slice(stock);
        let end1 = (body.bytes.len() - first - 8) as i64;
        body.bytes.extend_from_slice(guest);
        let end2 = (body.bytes.len() - second - 8) as i64;
        body.bytes.extend_from_slice(casino);
        body.bytes[first..first + 8].copy_from_slice(&end1.to_le_bytes());
        body.bytes[second..second + 8].copy_from_slice(&end2.to_le_bytes());
        let mut w = Writer::default();
        w.str("COM3D2_SAVE");
        w.i32(1040);
        w.str(time);
        w.i32(12);
        w.str("支配人");
        w.i32(3);
        w.str(comment);
        w.bytes.extend_from_slice(&body.bytes);
        (w.bytes, body.bytes)
    }
    #[test]
    fn native_stock_roster_round_trips_two_employees_to_the_script_boundary() -> Result<()> {
        // Original 1.0.4 MaidProp.Serialize on a detached sample with two
        // sub-prop slots and a non-default bone length.
        let hex = concat!(
            "0B434D3344325F4D50524F50100400000000000004776561720300000000000000",
            "00000000FDFFFFFF000000000B73616D706C652E6D656E751F0000000164000000",
            "FFFFFFFF0200000000010008616C742E6D656E750C0000000000003F00000000",
            "000000000000000001000000020000001100000001000000056A6F696E740000A03F"
        );
        let prop = hex
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect::<Vec<_>>();
        let write_props = |w: &mut Writer| {
            w.str("CM3D2_MPROP_LIST");
            w.i32(1040);
            w.i32(1);
            w.str("wear");
            w.bytes.extend_from_slice(&prop);
        };
        let mut stock = Writer::default();
        stock.i32(1);
        write_props(&mut stock);
        stock.str("CM3D2_MAID_MISC");
        stock.i32(1040);
        stock.i32(-1);
        stock.i32(3);
        stock.bytes.extend_from_slice(&[1, 2, 3]);
        stock.str("架空");
        for value in [0.25, 0.5, 0.75, 1.0] {
            stock.f32(value);
        }
        stock.i32(53);
        stock.i32(2);
        for (index, name) in ["第一", "第二"].into_iter().enumerate() {
            write_props(&mut stock);
            stock.str("CM3D2_MULTI_COL");
            stock.i32(1040);
            stock.i32(9);
            for i in 0..9 {
                stock.bool(i == 8);
                for n in 0..9 {
                    stock.i32(i * 10 + n);
                }
            }
            stock.str("CM3D2_MAID_STATUS");
            stock.i32(1040);
            stock.str(&format!("fixture-{index}"));
            stock.str("20261125000000");
            stock.bytes.extend_from_slice(&0i16.to_le_bytes()); // Original heroine.
            stock.bool(false);
            stock.i32(1); // Employment day.
            for text in [name, "社員", "", ""] {
                stock.str(text);
            }
            // Initial and current experience types, personality and contract.
            stock.bytes.extend_from_slice(&[0; 10]);
            stock.bytes.extend_from_slice(&[0; 10]); // Five body measurements.
            stock.str("");
            stock.bytes.extend_from_slice(&[0; 2 + 26 * 2]); // Relation and u16 parameters.
            stock.i64(1_234_567_890_123); // Sales.
            stock.i64(1_234_567_890_124); // Total sales.
            stock.bytes.extend_from_slice(&0xf032u16.to_le_bytes()); // Evaluation.
            stock.i64(4);
            stock.bytes.extend_from_slice(&[0; 3 + 2 + 8 + 2]); // Flags, eye tab and shifts.
            stock.bytes.extend_from_slice(&[0; 12 * 2]); // Counts for the twelve collections.
            stock.bytes.extend_from_slice(&[0; 3]); // Optional old status and chosen classes.
            stock.u32(2_177_629_204);
            stock.str("CM3D2_MAID_MISC");
            stock.i32(1040);
            stock.i32(index as i32);
            stock.i32(0);
            stock.str("");
            for _ in 0..4 { stock.f32(1.); }
            stock.i32(51);
            stock.str("CM3D2_MAID_BODY");
            stock.i32(1040);
        }
        stock.str("CM3D2_SCRIPT");
        stock.i32(1040);
        let fixture = |bytes: &[u8]| slot_with_stock("20261125000000", "", bytes).0;
        let bytes = fixture(&stock.bytes);
        let parsed = crate::save::stock_1040::read_slot(&bytes)?;
        let man = &parsed.men[0];
        assert_eq!(man.properties, [("wear".into(), std::borrow::Cow::Borrowed(prop.as_slice()))]);
        assert_eq!(man.misc.active_slot, -1);
        assert_eq!(man.misc.thumbnail.as_ref(), [1, 2, 3]);
        assert_eq!(man.misc.thumbnail_time, "架空");
        assert_eq!(man.misc.color, [0.25, 0.5, 0.75, 1.]);
        assert_eq!(man.misc.voice_pitch, 53);
        assert_eq!(parsed.employees.len(), 2);
        let first = &parsed.employees[0];
        assert_eq!(first.properties, man.properties);
        assert!(first.colors[8].used);
        assert_eq!(first.colors[8].shadow_contrast, 88);
        assert_eq!(parsed.employees[1].misc.active_slot, 1);
        assert_eq!(
            crate::binio::Reader::new(&bytes[parsed.script_offset..]).str()?,
            "CM3D2_SCRIPT"
        );
        let chr = parsed.encode()?;
        assert_eq!(chr.as_slice(), &bytes[parsed.script_offset - chr.len()..parsed.script_offset]);
        // Relative bookmarks still point inside the file, but no part may read
        // the following management section to complete a truncated field.
        for len in 0..stock.bytes.len() {
            assert!(
                crate::save::stock_1040::read_slot(&fixture(&stock.bytes[..len])).is_err(),
                "truncated at {len}"
            );
        }
        let (empty, _) = slot("20261125000000", "");
        let parsed = crate::save::stock_1040::read_slot(&empty)?;
        assert!(parsed.employees.is_empty() && parsed.men.is_empty());
        assert_eq!(
            crate::binio::Reader::new(&empty[parsed.script_offset..]).str()?,
            "CM3D2_SCRIPT"
        );
        Ok(())
    }

    const CHECKPOINT: [u8; 4] = [0x80, 0x00, 0x72, 0x45];

    struct NativeSlotSections {
        management: Vec<u8>,
        guest: Vec<u8>,
        casino: Vec<u8>,
    }

    fn native_slot_sections() -> Result<NativeSlotSections> {
        let mut management = Writer::default();
        management.i32(0); // Men.
        management.i32(0); // Employees.
        for header in ["CM3D2_SCRIPT", "CM3D2_KAG"] {
            management.str(header);
            management.i32(1040);
        }
        management.i32(CHECKPOINT.len() as i32);
        management.bytes.extend(CHECKPOINT);
        management.bool(true);
        management.bool(false);
        management.str("CM3D2_DeskCustomize");
        management.i32(1040);
        management.i32(1);
        management.i32(20);
        management.i32(21);
        management.bool(false);
        management.bool(true);
        for n in [4.5, -1.25, 0.0, -15.0, 180.0, 0.0, 1.0, 2.0, 1.0] {
            management.f32(n);
        }
        management.str("COM3D2_SCENARIO_SELECT");
        management.f32(1.3);
        for value in [
            "CM3D2_SCENARIO_ID_START",
            "870",
            "40",
            "CM3D2_SCENARIO_ID_END",
            "CM3D2_SELECTABLE_START",
            "40",
            "CM3D2_SELECTABLE_END",
        ] {
            management.str(value);
        }
        management.bool(true);
        management.bool(false);
        management.str("CM3D21_FACILITY_MGR");
        management.i32(1040);
        management.str("5");
        management.i32(0); // Existing rooms.
        management.str("CM3D21_TEMP_FLAG");
        management.i32(0); // Materials.
        management.i32(0); // Pending rooms.
        for _ in 0..3 {
            management.str("CM3D21_TEMP_FLAG");
            management.i32(0);
        }

        // The installed 1.0.4 SaveData.Serialize and DataArray.Serialize
        // generated this nested guest record using synthetic flags and text.
        let reference = concat!(
            "07000000050000000e32303236313132353030303030300a0000000c00000006",
            "e68ea5e5aea2ce0110434d334432315f54454d505f464c41470100000004726f",
            "6f6d0766697874757265010000000e666978747572652d776f726b657210434d",
            "334432315f54454d505f464c41470100000007636f6d6d656e747be4bbaee4bb",
            "aee4bbaee4bbaee4bbaee4bbaee4bbaee4bbaee4bbaee4bbaee4bbaee4bbaee4",
            "bbaee4bbaee4bbaee4bbaee4bbaee4bbaee4bbaee4bbaee4bbaee4bbaee4bbae",
            "e4bbaee4bbaee4bbaee4bbaee4bbaee4bbaee4bbaee4bbaee4bbaee4bbaee4bb",
            "aee4bbaee4bbaee4bbaee4bbaee4bbaee4bbaee4bbae",
        );
        let mut guest = Writer::default();
        guest.str("COM3D2_KASIZUKI");
        guest.i32(1040);
        guest.bytes.push(1);
        for i in (0..reference.len()).step_by(2) {
            guest.bytes.push(u8::from_str_radix(&reference[i..i + 2], 16)?);
        }

        let mut casino = Writer::default();
        for word in ["COM3D2_CASINO", "Ver:1.1", "Low"] {
            casino.str(word);
        }
        casino.i32(-1);
        casino.i32(2);
        for word in ["High", "COM3D2_CASINOSHOP_ID", "32", "29", "COM3D2_CASINOSHOP_ID_END"] {
            casino.str(word);
        }
        casino.bool(true);
        casino.bool(false);

        Ok(NativeSlotSections {
            management: management.bytes,
            guest: guest.bytes,
            casino: casino.bytes,
        })
    }

    #[test]
    fn native_slot_round_trips_all_sections_and_relocates_bookmarks() -> Result<()> {
        use crate::save::slot_1040::Slot;
        let NativeSlotSections { management, guest, casino } = native_slot_sections()?;
        let slot = |management: &[u8], guest: &[u8], casino: &[u8]| {
            slot_with_sections("20261125000000", "fixture", management, guest, casino).0
        };
        let bytes = slot(&management, &guest, &casino);
        let mut snapshot = Slot::read(&bytes)?;
        assert_eq!(snapshot.csv_paths[0], ["base"]);
        assert_eq!(snapshot.management.script.checkpoint, CHECKPOINT);
        assert!(snapshot.management.script.fade_wait && !snapshot.management.script.enabled);
        let item = &snapshot.management.desk[0];
        assert_eq!((item.key, item.id), (20, 21));
        assert!(item.seasonal && !item.visible);
        assert_eq!(item.position, [4.5, -1.25, 0.0]);
        assert_eq!(item.rotation, [-15.0, 180.0, 0.0]);
        assert_eq!(snapshot.management.scenario.played, [(870, true), (40, false)]);
        assert_eq!(snapshot.management.scenario.selectable, [40]);
        assert!(snapshot.management.facilities.rooms.is_empty());
        assert_eq!(snapshot.guest_slots[0].slot, 7);
        assert_eq!(snapshot.guest_slots[0].play_count, 12);
        assert_eq!(snapshot.guest_slots[0].characters[0].1[0].1, "仮".repeat(41));
        assert_eq!(snapshot.casino.low.as_ref().unwrap().stock_index, 2);
        assert!(snapshot.casino.high.is_none());
        assert_eq!(snapshot.casino.sold_out, [(32, true), (29, false)]);
        assert_eq!(snapshot.encode()?, bytes);

        snapshot.header.comment = "別のコメント".into();
        snapshot.guest_slots[0].comment = "再開用".repeat(95);
        snapshot.casino.sold_out[0].1 = false;
        let changed = snapshot.encode()?;
        let restored = Slot::read(&changed)?;
        assert_eq!(restored.header.comment, "別のコメント");
        assert_eq!(restored.guest_slots, snapshot.guest_slots);
        assert_eq!(restored.casino, snapshot.casino);

        let short_management = slot(&management[..management.len() - 1], &guest, &casino);
        assert!(Slot::read(&short_management).is_err());
        let short_guest = slot(&management, &guest[..guest.len() - 1], &casino);
        assert!(Slot::read(&short_guest).is_err());
        assert!(Slot::read(&bytes[..bytes.len() - 1]).is_err());
        Ok(())
    }

    #[test]
    #[ignore = "requires COM3D2_ROOT standalone installation"]
    fn installed_daily_slot_modes_and_confirmed_deletion() -> Result<()> {
        let (_tmp, mut rt, c, mut op, _, _, mut assets) = daily::installed_panel(&[])?;
        let NativeSlotSections { management, guest, casino } = native_slot_sections()?;
        let (bytes, _) = slot_with_sections("20261125000000", "保管", &management, &guest, &casino);
        op.storage.write_atomic(&save_path(13)?, &bytes)?;
        op.storage
            .write_atomic(&save_path(0)?, &slot("20261124000000", "別のデータ").0)?;
        let storage = op.storage.clone();
        let mut viewer = Viewer::open(Mode::DailySave, &mut assets, &mut op, &mut rt.host, c, 0)?;
        let mut widgets = viewer.populate(assets.slot_table)?;
        let mut now = 500;
        let mut click =
            |v: &mut Viewer, w: &mut Vec<Widget>, suffix: &str, button: u64| -> Result<bool> {
                let rect = w
                    .iter()
                    .find(|w| w.path.ends_with(suffix) && w.collider.is_some())
                    .with_context(|| format!("no clickable {suffix}"))?
                    .collider
                    .unwrap();
                let mut input = InputFrame {
                    pointer_x: (rect[0] + rect[2] * 0.10) as i32,
                    pointer_y: (rect[1] + rect[3] * 0.5) as i32,
                    buttons_pressed: button,
                    ..Default::default()
                };
                v.input(w, &mut assets, &mut op, &mut rt.host, c, input, 0, now)?;
                input.buttons_pressed = 0;
                v.input(
                    w,
                    &mut assets,
                    &mut op,
                    &mut rt.host,
                    c,
                    input,
                    button,
                    now + 20,
                )?;
                now += 600;
                v.input(
                    w,
                    &mut assets,
                    &mut op,
                    &mut rt.host,
                    c,
                    InputFrame::default(),
                    0,
                    now,
                )
            };
        let message = |w: &[Widget], msg: &str| {
            w.iter()
                .any(|w| w.label.as_ref().is_some_and(|l| l.text.contains(msg)))
        };
        assert_eq!(viewer.page, 2);
        assert!(widgets.iter().any(|w| w.path.ends_with("/TitleSave")));
        click(
            &mut viewer,
            &mut widgets,
            "serialNo_10/EmptyUnit",
            buttons::MOUSE_LEFT,
        )?;
        assert!(message(&widgets, "データを保存しますか？"));
        click(
            &mut viewer,
            &mut widgets,
            "SystemDialog/Base/Cancel",
            buttons::MOUSE_LEFT,
        )?;
        click(
            &mut viewer,
            &mut widgets,
            "serialNo_13/DataUnit",
            buttons::MOUSE_LEFT,
        )?;
        assert!(message(&widgets, "上書きして保存しますが宜しいですか？"));
        click(
            &mut viewer,
            &mut widgets,
            "SystemDialog/Base/Ok",
            buttons::MOUSE_LEFT,
        )?;
        assert_eq!(viewer.operation.take(), Some(SlotOperation::Save {
            slot: 13,
            comment: "保管".into(),
        }));
        click(
            &mut viewer,
            &mut widgets,
            "/DisplayLoad",
            buttons::MOUSE_LEFT,
        )?;
        assert_eq!(viewer.mode, Mode::DailyLoad);
        assert!(widgets.iter().any(|w| w.path.ends_with("/TitleLoad")));
        click(
            &mut viewer,
            &mut widgets,
            "serialNo_10/EmptyUnit",
            buttons::MOUSE_LEFT,
        )?;
        assert!(viewer.dialog.is_none());
        click(
            &mut viewer,
            &mut widgets,
            "serialNo_13/DataUnit",
            buttons::MOUSE_LEFT,
        )?;
        assert!(message(&widgets, "データをロードしますか？"));
        click(
            &mut viewer,
            &mut widgets,
            "SystemDialog/Base/Ok",
            buttons::MOUSE_LEFT,
        )?;
        assert_eq!(viewer.operation.take(), Some(SlotOperation::Load { slot: 13 }));
        click(
            &mut viewer,
            &mut widgets,
            "serialNo_13/DataUnit",
            buttons::MOUSE_RIGHT,
        )?;
        assert!(message(&widgets, "データを削除しますが宜しいですか。"));
        click(
            &mut viewer,
            &mut widgets,
            "SystemDialog/Base/Cancel",
            buttons::MOUSE_LEFT,
        )?;
        assert_eq!(viewer.entries[&13].comment, "保管");
        assert_eq!(storage.read(&save_path(13)?)?, bytes);
        click(
            &mut viewer,
            &mut widgets,
            "serialNo_13/DataUnit",
            buttons::MOUSE_RIGHT,
        )?;
        click(
            &mut viewer,
            &mut widgets,
            "SystemDialog/Base/Ok",
            buttons::MOUSE_LEFT,
        )?;
        assert!(!viewer.entries.contains_key(&13));
        assert!(widgets.iter().any(|w| w.path == "serialNo_13/EmptyUnit"));
        assert!(click(
            &mut viewer,
            &mut widgets,
            "/Cancel",
            buttons::MOUSE_LEFT
        )?);
        assert_eq!(
            catalog(op.storage.as_ref())?
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [0]
        );
        Ok(())
    }
    #[test]
    #[ignore = "requires COM3D2_ROOT standalone installation"]
    fn installed_native_slot_catalog_and_comment() -> Result<()> {
        let (_tmp, mut rt, c, mut op, _, _, mut assets) = daily::installed_panel(&[])?;
        assert!(catalog(op.storage.as_ref())?.is_empty());
        let (older, _) = slot("20261124235959", "前日");
        let (newer, body) = slot("20261125112233", "旧メモ");
        op.storage.write_atomic(&save_path(0)?, &older)?;
        op.storage.write_atomic(&save_path(24)?, &newer)?;
        let mut v = Viewer::open(Mode::TitleLoad, &mut assets, &mut op, &mut rt.host, c, 100)?;
        assert_eq!((v.page, v.latest), (3, Some(24)));
        let mut widgets = v.populate(assets.slot_table)?;
        for name in ["支配人", "3", "12", "2026.11.25 11:22", "旧メモ"] {
            assert!(
                widgets.iter().any(|w| w.path.starts_with("serialNo_24/")
                    && w.label.as_ref().is_some_and(|l| l.text == name)),
                "missing visible field {name}"
            );
        }
        assert_eq!(
            widgets
                .iter()
                .filter(|w| w.path.ends_with("/EmptyUnit") && w.collider.is_some())
                .count(),
            9
        );
        let pos = |widgets: &[Widget], suffix: &str| -> Result<InputFrame> {
            let r = widgets
                .iter()
                .find(|w| w.path.ends_with(suffix) && w.collider.is_some())
                .with_context(|| format!("missing slot control {suffix}"))?
                .collider
                .unwrap();
            let frame = InputFrame {
                pointer_x: (r[0] + r[2] / 2.) as i32,
                pointer_y: (r[1] + r[3] / 2.) as i32,
                ..Default::default()
            };
            ensure!(
                (0..1920).contains(&frame.pointer_x) && (0..1080).contains(&frame.pointer_y),
                "{suffix} is offscreen {r:?}"
            );
            Ok(frame)
        };
        assert_eq!(
            v.hit(&widgets, &pos(&widgets, "serialNo_20/EmptyUnit")?),
            None
        );
        assert_eq!(
            v.hit(&widgets, &pos(&widgets, "/InputField")?),
            Some(Action::Comment(24))
        );
        let mut page = pos(&widgets, "/Page_1")?;
        page.buttons_pressed = buttons::MOUSE_LEFT;
        v.input(
            &mut widgets,
            &mut assets,
            &mut op,
            &mut rt.host,
            c,
            page,
            0,
            600,
        )?;
        page.buttons_pressed = 0;
        v.input(
            &mut widgets,
            &mut assets,
            &mut op,
            &mut rt.host,
            c,
            page,
            buttons::MOUSE_LEFT,
            630,
        )?;
        assert_eq!(v.page, 1);
        let old = &v.entries[&24];
        let changed = change_comment(op.storage.as_ref(), 24, old, &"猫".repeat(30))?;
        assert_eq!(changed.comment, "猫".repeat(25));
        assert!(change_comment(op.storage.as_ref(), 24, old, "stale").is_err());
        let data = op.storage.read(&save_path(24)?)?;
        let mut reader = std::io::Cursor::new(&data);
        assert_eq!(read_header(&mut reader)?, changed);
        assert_eq!(&data[reader.position() as usize..], &body);
        let (player, offset) = crate::save::player_1040::read_slot_player(&data)?;
        assert_eq!((player.days, player.money), (12, 1_234_567));
        assert_eq!(&data[offset..offset + 8], &[0; 8]);
        assert_eq!(op.storage.read(&save_path(0)?)?, older);
        assert_eq!(catalog(op.storage.as_ref())?[&24].comment, changed.comment);
        let mut truncated = data.clone();
        truncated.truncate(truncated.len() - 5);
        op.storage.write_atomic(&save_path(24)?, &truncated)?;
        assert!(change_comment(op.storage.as_ref(), 24, &changed, "invalid").is_err());
        assert_eq!(op.storage.read(&save_path(24)?)?, truncated);
        Ok(())
    }
}
