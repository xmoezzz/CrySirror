// SPDX-License-Identifier: MPL-2.0
//! Native Kasizuki room and scenario selection.
use super::*;
use anyhow::ensure;
use crate::save::slot_1040::Guest;

pub(crate) struct Room {
    pub id: i32,
    pub name: String,
    pub description: String,
    pub(crate) man: i32,
}

pub(crate) struct Play {
    pub(crate) id: i32,
    pub name: String,
    pub description: String,
    pub conditions: Vec<String>,
    pub(crate) file: String,
    required_likability: i32,
}

pub(crate) const PROTAGONIST: i32 = 10;
const COMMENT: &str = "フリーコメント";

pub(crate) struct ManProfile {
    pub id: i32,
    pub title: String,
    pub preset: String,
    pub comment: String,
    pub favorite: String,
}

/// Kasizuki.ManData.Data: list name, NPC preset key, comment and favorite
/// are separate columns of the enabled ManData records.
pub(crate) fn man_profiles(s: &mut TjsState) -> Result<Vec<ManProfile>> {
    let table = s.retail.table("kasizuki_man_list.nei")?;
    let mut rows = Vec::new();
    for r in 1..table.rows {
        let id = table.int(0, r);
        if table.exists(0, r)
            && (s.player.flag("__lockNTRPlay") == 0 || id == PROTAGONIST)
            && s.retail.enabled("kasizuki_man_enabled_list", id)?
        {
            rows.push(ManProfile {
                id,
                title: table.string(2, r),
                preset: table.string(3, r),
                comment: table.string(4, r),
                favorite: table.string(5, r),
            });
        }
    }
    Ok(rows)
}

pub(crate) fn men(s: &mut TjsState) -> Result<Vec<(i32, String)>> {
    Ok(man_profiles(s)?.into_iter().map(|p| (p.id, p.title)).collect())
}

fn man_column(s: &mut TjsState, table: &Table, man: i32, from: usize) -> Result<usize> {
    let men = s.retail.table("kasizuki_man_list.nei")?;
    let name = men.string(1, row_id(&men, man)?);
    (from..table.columns)
        .find(|&i| table.string(i, 0) == name)
        .with_context(|| format!("guest man column {name}"))
}

pub(crate) fn man(s: &TjsState) -> i32 {
    number(&s.retail, "男種類")
}

pub(crate) fn use_man(s: &mut TjsState, id: i32) -> Result<()> {
    ensure!(
        man_profiles(s)?.iter().any(|profile| profile.id == id),
        "unavailable guest man {id}"
    );
    s.retail
        .kasizuki
        .system
        .insert("男種類".into(), id.to_string());
    Ok(())
}

pub(crate) struct MaidOverview {
    pub original: crate::params::MaidStatus,
    pub personality: String,
    pub work_count: i32,
    pub comment: String,
    pub work_locked: bool,
    pub affinity_locked: bool,
}

pub(crate) fn overview(s: &mut TjsState, id: usize) -> Result<MaidOverview> {
    let maid = s.maids.get(&id).context("missing guest character")?;
    let personalities = s.retail.table("maid_status_personal_list.nei")?;
    let row = row_id(&personalities, maid.personal)?;
    let record = &s.retail.kasizuki;
    Ok(MaidOverview {
        original: maid.clone(),
        personality: personalities.string(2, row),
        work_count: record.maids.get(&maid.guid_id)
            .and_then(|m| m.get("仕事回数")).and_then(|v| v.parse().ok()).unwrap_or_default(),
        comment: record.maids.get(&maid.guid_id).and_then(|m| m.get(COMMENT)).cloned().unwrap_or_default(),
        work_locked: parameter_locked(&s.retail, "仕事回数ロック"),
        affinity_locked: man(s) == PROTAGONIST || parameter_locked(&s.retail, "好感度ロック"),
    })
}

pub(crate) fn set_comment(s: &mut TjsState, id: usize, comment: String) -> Result<()> {
    let maid = s.maids.get(&id).context("missing guest comment character")?;
    s.retail.kasizuki.maids.entry(maid.guid_id.clone()).or_default().insert(COMMENT.into(), comment);
    Ok(())
}

fn parameter_locked(s: &State, key: &str) -> bool {
    s.kasizuki.system.get(key).is_some_and(|v| v.eq_ignore_ascii_case("true"))
}

pub(crate) fn toggle_parameter_lock(s: &mut TjsState, affinity: bool) {
    if affinity && man(s) == PROTAGONIST {
        return;
    }
    let key = if affinity { "好感度ロック" } else { "仕事回数ロック" };
    let locked = !parameter_locked(&s.retail, key);
    s.retail.kasizuki.system.insert(key.into(), locked.to_string());
}

pub(crate) fn rooms(s: &mut TjsState, man: i32, built_only: bool) -> Result<Vec<Room>> {
    let table = s.retail.table("kasizuki_room_list.nei")?;
    let column = man_column(s, &table, man, 9)?;
    let mut rows = Vec::new();
    for r in 1..table.rows {
        let id = table.int(0, r);
        if !table.exists(0, r)
            || table.string(column, r) != "○"
            || !s.retail.enabled("kasizuki_room_enabled_list", id)?
            || (built_only && !s.retail.has_built_facility(table.int(3, r)))
        {
            continue;
        }
        rows.push(Room {
            id,
            name: table.string(5, r),
            description: table.string(7, r),
            man,
        });
    }
    Ok(rows)
}

pub(crate) fn replay_rooms(s: &mut TjsState) -> Result<Vec<Room>> {
    let rooms = s.retail.table("kasizuki_room_list.nei")?;
    let facilities = s.retail.table("facility.nei")?;
    let locked = s.player.flag("__lockNTRPlay") != 0;
    let protagonist = man_column(s, &rooms, 10, 9)?;
    let mut output = Vec::new();
    for r in 1..rooms.rows {
        let id = rooms.int(0, r);
        if !rooms.exists(0, r) || !s.retail.enabled("kasizuki_room_enabled_list", id)? {
            continue;
        }
        let facility = row_id(&facilities, rooms.int(3, r)).or_else(|_| {
            let name = rooms.string(4, r);
            (1..facilities.rows)
                .find(|&i| facilities.string(2, i) == name)
                .with_context(|| format!("guest facility for room {id}"))
        })?;
        let condition = facilities.string(17, facility);
        let visible = if locked {
            rooms.string(protagonist, r) == "○"
                && matches!(condition.as_str(), "" | "○" | "〇")
                && subordinate(s, &rooms, id)?.is_none()
                && !plays(s, id, 10, false)?.is_empty()
        } else {
            rooms.exists(2, r) && !matches!(condition.as_str(), "○" | "〇")
        };
        if visible {
            output.push(Room {
                id,
                name: rooms.string(5, r),
                description: rooms.string(7, r),
                man: man(s),
            });
        }
    }
    Ok(output)
}

pub(crate) fn replay_men(s: &mut TjsState, room: i32) -> Result<Vec<(i32, String)>> {
    let rooms = s.retail.table("kasizuki_room_list.nei")?;
    let row = row_id(&rooms, room)?;
    let mut output = Vec::new();
    for (id, name) in men(s)? {
        let column = man_column(s, &rooms, id, 9)?;
        if rooms.string(column, row) == "○" {
            output.push((id, name));
        }
    }
    Ok(output)
}

pub(crate) fn eligible_maids(s: &TjsState) -> Vec<usize> {
    s.stock
        .iter()
        .copied()
        .filter(|id| {
            s.maids.get(id).is_some_and(|m| {
                s.retail
                    .characters
                    .get(&m.guid_id)
                    .is_none_or(|status| status.old.is_none() && status.sub_name.is_none())
            })
        })
        .collect()
}

fn subordinate(s: &mut TjsState, rooms: &Table, room: i32) -> Result<Option<i32>> {
    let mut only = None;
    for r in 1..rooms.rows {
        if !rooms.exists(0, r) || !rooms.exists(2, r) || rooms.int(2, r) != room {
            continue;
        }
        let id = rooms.int(0, r);
        if s.retail.enabled("kasizuki_room_enabled_list", id)? && only.replace(id).is_some() {
            return Ok(None);
        }
    }
    Ok(only)
}

fn room_keys(
    s: &mut TjsState,
    rooms: &Table,
    room: i32,
    include_subordinate: bool,
) -> Result<BTreeSet<String>> {
    let mut all = BTreeSet::new();
    let mut depth = room;
    loop {
        let key = rooms.string(1, row_id(rooms, depth)?);
        ensure!(all.insert(key), "cyclic guest room hierarchy");
        if !include_subordinate {
            break;
        }
        let Some(child) = subordinate(s, rooms, depth)? else {
            break;
        };
        depth = child;
    }
    Ok(all)
}

pub(crate) fn plays(
    s: &mut TjsState,
    room: i32,
    man: i32,
    include_subordinate: bool,
) -> Result<Vec<Play>> {
    let rooms = s.retail.table("kasizuki_room_list.nei")?;
    let all = room_keys(s, &rooms, room, include_subordinate)?;
    let counts = s.retail.table("kasizuki_work_count_convert_list.nei")?;
    let tiers: BTreeSet<_> = (1..counts.rows).map(|i| counts.int(0, i)).collect();
    let table = s.retail.table("kasizuki_play_list.nei")?;
    let column = man_column(s, &table, man, 20 + tiers.len())?;
    let mut rows = Vec::new();
    for r in 1..table.rows {
        let id = table.int(0, r);
        if !table.exists(0, r)
            || !all.contains(&table.string(1, r))
            || table.string(column, r) != "○"
            || !s.retail.enabled("kasizuki_play_enabled_list", id)?
        {
            continue;
        }
        rows.push(Play {
            id,
            file: table.string(2, r),
            name: table.string(3, r),
            description: table.string(4, r),
            conditions: (5..10)
                .filter(|&column| table.exists(column, r))
                .map(|column| table.string(column, r))
                .collect(),
            required_likability: table.int(16, r),
        });
    }
    rows.sort_by_key(|r| r.id);
    Ok(rows)
}

pub(crate) fn room_roster(s: &mut TjsState, room: i32, man: i32) -> Result<Vec<usize>> {
    let plays = plays(s, room, man, true)?;
    let mut output = Vec::new();
    for id in eligible_maids(s) {
        for p in &plays {
            if character::eligible_for(s, id, p.id, man)? {
                output.push(id);
                break;
            }
        }
    }
    Ok(output)
}

/// Highest satisfied likability requirement, in the authored ID order.
pub(crate) fn optimal(s: &mut TjsState, maid: usize, room: i32, man: i32) -> Result<Play> {
    let mut result: Option<Play> = None;
    for p in plays(s, room, man, true)? {
        if character::eligible_for(s, maid, p.id, man)?
            && result
                .as_ref()
                .is_none_or(|v| p.required_likability > v.required_likability)
        {
            result = Some(p);
        }
    }
    result.context("no eligible guest scenario")
}

pub(crate) fn select_room(s: &mut TjsState, room: i32) {
    s.retail
        .kasizuki
        .system
        .insert("選択部屋".into(), room.to_string());
}

pub(crate) fn post_scenario(s: &mut TjsState) {
    let locked_work = parameter_locked(&s.retail, "仕事回数ロック");
    let locked_affinity = parameter_locked(&s.retail, "好感度ロック");
    let guest = man(s) == 20;
    if let Some(maid) = s.maid(0) {
        let guid = maid.guid_id.clone();
        let record = s.retail.kasizuki.maids.entry(guid).or_default();
        if !locked_work {
            let count = record.entry("仕事回数".into()).or_default();
            *count = count.parse::<i32>().unwrap_or_default().wrapping_add(1).to_string();
        }
        if !locked_affinity && guest {
            let affinity = record.entry("好感度＿傅き男１".into()).or_default();
            *affinity = affinity.parse::<i32>().unwrap_or_default().wrapping_add(4).to_string();
        }
        let count = number(&s.retail, "プレイ回数").wrapping_add(1);
        s.retail
            .kasizuki
            .system
            .insert("プレイ回数".into(), count.to_string());
    }
}

/// Select an existing guest checkpoint, or initialize a new guest on the
/// same campaign roster. Original ResetData preserves the campaign state.
pub(crate) fn apply_slot(s: &mut TjsState, origin: i32, saved: Option<&Guest>) -> Result<()> {
    let mut guest = Kasizuki::default();
    if let Some(saved) = saved {
        ensure!(
            man_profiles(s)?.iter().any(|profile| profile.id == saved.last_man_type),
            "guest save references an unavailable man {}", saved.last_man_type
        );
        guest.system = saved.system.iter().cloned().collect();
        guest.system.insert("男種類".into(), saved.last_man_type.to_string());
        let guids: BTreeSet<_> = s.stock.iter().filter_map(|id| s.maids.get(id)).map(|maid| &maid.guid_id).collect();
        guest.maids = saved.characters.iter().filter(|(guid, _)| guids.contains(guid))
            .map(|(guid, flags)| (guid.clone(), flags.iter().cloned().collect())).collect();
    }
    s.retail.kasizuki = guest;
    s.retail.native_guest_origin = Some(origin);
    s.player.generic_flag.insert("傅きモード".into(), 1);
    Ok(())
}

pub(crate) fn origin(s: &TjsState) -> Result<i32> {
    s.retail.native_guest_origin.context("guest mode has no selected campaign slot")
}

pub(crate) fn saved_slots(s: &TjsState) -> &[Guest] {
    &s.retail.native_guest_slots
}

pub(crate) fn saved_slots_mut(s: &mut TjsState) -> &mut [Guest] {
    &mut s.retail.native_guest_slots
}

pub(crate) fn set_saved_slots(s: &mut TjsState, slots: Vec<Guest>) {
    s.retail.native_guest_slots = slots;
}

pub(crate) fn capture_slot(s: &TjsState, slot: i32, comment: String) -> Guest {
    let pairs = |flags: &BTreeMap<String, String>| flags.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<Vec<_>>();
    let records = &s.retail.kasizuki;
    Guest {
        slot,
        save_time: Local::now().format("%Y%m%d%H%M%S").to_string(),
        last_man_type: man(s),
        play_count: number(&s.retail, "プレイ回数"),
        comment,
        system: pairs(&records.system),
        characters: records.maids.iter().map(|(guid, flags)| (guid.clone(), pairs(flags))).collect(),
    }
}
