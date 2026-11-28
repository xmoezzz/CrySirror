// SPDX-License-Identifier: MPL-2.0
use super::*;

/// Each native playing slot has a base skill and an optional character-owned
/// skill. Free and VR selections have a base record without owned skill data.
#[derive(Clone, Default)]
pub(super) struct Manager {
    pub legacy: bool,
    pub free: bool,
    pub script_call: bool,
    pub selected: Vec<Option<(i32, Option<i32>)>>,
    pub free_choice: Option<super::free_skill::Choice>,
}
impl State {
    pub(crate) fn yotogi_profile(&self) -> Option<bool> {
        self.selection.as_ref().map(|manager| manager.legacy)
    }

    pub(crate) fn scene_loaded(&mut self, name: &str, tag: &BTreeMap<String, String>) {
        self.vr_karaoke = name.contains("SceneVRCommunication").then(|| {
            tag.get("name")
                .is_some_and(|n| n.contains("SceneVRCommunication"))
                && tag.contains_key("karaoke")
        });
        self.selection = matches!(name, "SceneYotogi" | "SceneYotogiOld").then(|| Manager {
            legacy: name == "SceneYotogiOld",
            free: tag.contains_key("free_start"),
            script_call: tag.get("name").is_some_and(|n| n == name),
            selected: vec![None; 7],
            free_choice: None,
        });
    }
}

pub(super) fn selected(s: &mut TjsState, a: &[Variant]) -> Result<Variant> {
    arity(a, 1)?;
    let Some(m) = s
        .retail
        .selection
        .as_ref()
        .filter(|m| !m.legacy && !m.selected.is_empty())
    else {
        return Ok(Variant::Void);
    };
    Ok(boolean(
        m.selected
            .iter()
            .flatten()
            .any(|(_, owned)| *owned == Some(arg_int(a, 0) as i32)),
    ))
}

pub(super) fn special(s: &mut TjsState, _: &[Variant]) -> Result<Variant> {
    let Some(m) = s
        .retail
        .selection
        .as_ref()
        .filter(|m| !m.legacy && !m.selected.is_empty())
    else {
        return Ok(Variant::Void);
    };
    let id = m.selected[0]
        .context("first playing slot has no base skill")?
        .0;
    let t = s.retail.table("yotogi_skill_list.nei")?;
    let r = row_id(&t, id)?;
    let cell = t.string(23, r);
    let n = if cell.is_empty() {
        0
    } else {
        cell.trim().parse::<i32>()?
    };
    let text = ["Null", "Drunk", "Mask", "Drug", "Faint", "Confess"]
        .get(n as usize)
        .map(|n| n.to_string())
        .unwrap_or_else(|| n.to_string());
    Ok(Variant::String(text))
}

/// CharacterSelectManager receives its list and capacity from the active
/// Yotogi manager. The lead remains in slot zero and is excluded from the list.
pub(crate) fn sub_cast(s: &mut TjsState, old: bool) -> Result<(usize, Vec<usize>)> {
    if old {
        require_transferred_character(s)?;
    }
    let lead = s
        .active_maids
        .first()
        .copied()
        .flatten()
        .context("no active lead character")?;
    let manager = s
        .retail
        .selection
        .as_ref()
        .context("no active Yotogi manager")?;
    ensure!(manager.legacy == old, "Yotogi profile mismatch");
    let free = manager.free;
    let scheduled = manager.script_call && !free;
    let slots = if free {
        let maid = s.maids.get(&lead).context("no lead status")?;
        if old {
            s.retail
                .characters
                .get(&maid.guid_id)
                .map(|status| status.old_skill_ids.iter().copied().collect::<Vec<_>>())
                .unwrap_or_default()
        } else {
            maid.skill_data.iter().map(|(id, _)| *id).collect()
        }
    } else {
        manager
            .selected
            .iter()
            .flatten()
            .map(|(id, _)| *id)
            .collect()
    };
    let mut roster = if scheduled {
        let t = s.retail.table("schedule_work_night.nei")?;
        let night = s.player.flag("時間帯") == 3;
        facility::roster(s)
            .into_iter()
            .filter(|id| {
                let maid = &s.maids[id];
                let work = if night {
                    maid.night_work_id
                } else {
                    maid.noon_work_id
                };
                row_id(&t, work).is_ok_and(|r| t.string(6, r) == "HaveSex")
            })
            .collect::<Vec<_>>()
    } else {
        s.stock
            .iter()
            .copied()
            .filter(|id| {
                s.maids.get(id).is_some_and(|maid| {
                    !old || (!maid.is_rental_maid
                        && s.retail
                            .characters
                            .get(&maid.guid_id)
                            .is_some_and(|status| status.old.is_some()))
                })
            })
            .collect()
    };
    if scheduled && old && s.maids[&lead].is_rental_maid {
        roster.clear();
    }
    let largest = if slots.is_empty() {
        1
    } else {
        let (table, column) = if old {
            (
                Arc::new(s.retail.legacy_table("yotogi_skil_detail.nei")?),
                12,
            )
        } else {
            (s.retail.table("yotogi_skill_list.nei")?, 13)
        };
        let mut largest = 1;
        for id in slots {
            largest = largest.max(table.int(column, row_id(&table, id)?));
        }
        largest as usize
    };
    let count = largest.min(roster.len()).saturating_sub(1);
    roster.retain(|id| *id != lead);
    Ok((count, roster))
}

pub(super) fn sub_select(s: &mut TjsState, _: &[Variant]) -> Result<Variant> {
    let old = s.retail.compatibility || s.player.flag("2.0モード") == 1;
    let Some(m) = s.retail.selection.as_ref().filter(|m| m.legacy == old) else {
        return Ok(boolean(false));
    };
    let ids: Vec<i32> = if m.free {
        let maid = s.maid(0).context("empty active maid for free selection")?;
        if old {
            s.retail
                .characters
                .get(&maid.guid_id)
                .map(|c| c.old_skill_ids.iter().copied().collect())
                .unwrap_or_default()
        } else {
            maid.skill_data.iter().map(|(id, _)| *id).collect()
        }
    } else {
        m.selected.iter().flatten().map(|(id, _)| *id).collect()
    };
    if ids.is_empty() {
        return Ok(boolean(false));
    }
    let possible = if m.script_call && !m.free {
        let t = s.retail.table("schedule_work_night.nei")?;
        let night = s.player.flag("時間帯") == 3;
        facility::roster(s)
            .iter()
            .filter(|i| {
                let maid = &s.maids[i];
                let id = if night {
                    maid.night_work_id
                } else {
                    maid.noon_work_id
                };
                row_id(&t, id).is_ok_and(|r| t.string(6, r) == "HaveSex")
            })
            .count()
    } else if old {
        s.stock
            .iter()
            .filter_map(|i| s.maids.get(i))
            .filter(|m| {
                !m.is_rental_maid
                    && s.retail
                        .characters
                        .get(&m.guid_id)
                        .is_some_and(|c| c.old.is_some())
            })
            .count()
    } else {
        s.stock.iter().filter(|i| s.maids.contains_key(i)).count()
    };
    let (t, col) = if old {
        (
            Arc::new(s.retail.legacy_table("yotogi_skil_detail.nei")?),
            12,
        )
    } else {
        (s.retail.table("yotogi_skill_list.nei")?, 13)
    };
    for id in ids {
        let n = t.int(col, row_id(&t, id)?).max(1);
        if n > 1 && n as usize <= possible {
            return Ok(boolean(true));
        }
    }
    Ok(boolean(false))
}
