// SPDX-License-Identifier: MPL-2.0
//! FreeModeItemEveryday, FreeModeItemVip and ScheduleAPI's catalogue rules.
use super::*;

pub(crate) struct Replay {
    pub title: String,
    pub file: String,
    pub description: String,
    pub enabled: bool,
}

pub(crate) fn legacy_mode(s: &TjsState) -> bool {
    s.player.flag("2.0モード") == 1
}

fn ids(table: &Table) -> BTreeSet<i32> {
    (1..table.rows)
        .filter(|&row| table.exists(0, row))
        .map(|row| table.int(0, row))
        .collect()
}

pub(crate) fn scenes(s: &mut TjsState, story: bool) -> Result<Vec<Replay>> {
    let legacy = legacy_mode(s);
    if legacy
        && (story
            || !s
                .retail
                .linked_files()
                .exists("recollection_enabled_id_list.nei"))
    {
        return Ok(Vec::new());
    }
    let name = if story {
        "recollection_story.nei"
    } else {
        "recollection_normal2.nei"
    };
    let flag = if story {
        "シーン鑑賞_メイン_フラグ_"
    } else {
        "シーン鑑賞_一般_フラグ_"
    };
    let new = s.retail.table(name)?;
    let personal_id = s.maid(0).map(|m| m.personal);
    let sub_id = if !legacy && !story {
        let sub = s
            .maid(0)
            .and_then(|maid| s.retail.characters.get(&maid.guid_id))
            .and_then(|extra| extra.sub_name.clone());
        if let Some(sub) = sub {
            let table = s.retail.table("maid_status_submaid_list.nei")?;
            Some(
                (1..table.rows)
                    .find(|&row| table.string(1, row) == sub)
                    .map(|row| table.int(0, row))
                    .context("recollection sub-character id")?,
            )
        } else {
            None
        }
    } else {
        None
    };
    let personals = s.retail.table("maid_status_personal_list.nei")?;
    let personal = personal_id
        .and_then(|id| row_id(&personals, id).ok())
        .map(|row| personals.string(1, row));
    let locked = s.player.flag("__lockNTRPlay") != 0;
    let mut seen_ids = BTreeSet::new();
    let mut seen_flags = BTreeSet::new();
    let mut rows = BTreeMap::new();
    for row in 1..new.rows {
        let id = new.int(0, row);
        if !new.exists(0, row) || !s.retail.enabled("recollection_enabled_id_list", id)? {
            continue;
        }
        let key = new.string(3, row);
        if seen_ids.contains(&id) || !seen_flags.insert(key.clone()) {
            continue;
        }
        seen_ids.insert(id);
        if legacy || locked && new.string(4, row) == "○" {
            continue;
        }
        if !story
            && !sub_id.map_or_else(
                || {
                    personal.as_ref().is_some_and(|p| {
                        (16..new.columns)
                            .take_while(|&col| new.exists(col, 0))
                            .any(|col| new.string(col, 0) == *p && new.string(col, row) == "○")
                    })
                },
                |id| new.exists(15, row) && new.int(15, row) == id,
            )
        {
            continue;
        }
        let enabled = s.player.flag(&format!("{flag}{key}")) != 0;
        if enabled || !story {
            rows.insert(id, replay(&new, row, 5, enabled));
        }
    }
    if legacy {
        let allowed = s.retail.legacy_ids("recollection_enabled_id_list")?;
        let disabled = ids(s.retail.table("recollection_legacy_disable.nei")?.as_ref());
        let old = s.retail.legacy_table(name)?;
        for row in 1..old.rows {
            let id = old.int(0, row);
            let key = old.string(3, row);
            if !old.exists(0, row)
                || !allowed.contains(&id)
                || disabled.contains(&id)
                || seen_ids.contains(&id)
                || !seen_flags.insert(key.clone())
            {
                continue;
            }
            seen_ids.insert(id);
            let flag_name = format!("{flag}{key}");
            if s.player.flag(&flag_name) == 0
                && s.stock
                    .iter()
                    .filter_map(|id| s.maids.get(id))
                    .any(|maid| maid.flag(&key) != 0)
            {
                s.player.generic_flag.insert(flag_name.clone(), 1);
            }
            rows.insert(id, replay(&old, row, 4, s.player.flag(&flag_name) != 0));
        }
    }
    Ok(rows.into_values().collect())
}

fn replay(table: &Table, row: usize, description: usize, enabled: bool) -> Replay {
    Replay {
        title: table.string(1, row),
        file: table.string(2, row),
        description: table.string(description, row),
        enabled,
    }
}

pub(crate) struct Vip {
    pub work: i32,
    pub category: i32,
    pub icon: String,
    pub conditions: Vec<String>,
    pub replay: Replay,
}

pub(crate) fn vip(s: &mut TjsState) -> Result<(BTreeMap<i32, String>, Vec<Vip>)> {
    let legacy = legacy_mode(s);
    let locked = s.player.flag("__lockNTRPlay") != 0;
    let ntr = if locked {
        ids(s.retail.table("schedule_work_netorare.nei")?.as_ref())
    } else {
        BTreeSet::new()
    };
    let lead = s
        .active_maids
        .first()
        .copied()
        .flatten()
        .context("VIP requires an active character")?;
    let maid = s.maids.get(&lead).context("VIP lead status")?;
    let sub = s
        .retail
        .characters
        .get(&maid.guid_id)
        .and_then(|extra| extra.sub_name.clone());
    let personal = maid.personal;
    let main = s.retail.main_characters.contains(&lead);
    let personals = s.retail.table("maid_status_personal_list.nei")?;
    let personal = personals.string(1, row_id(&personals, personal)?);

    let categories = s.retail.table("schedule_work_night_category_list.nei")?;
    let mut labels = (1..categories.rows)
        .filter(|&row| categories.exists(0, row))
        .map(|row| (categories.int(0, row), categories.string(1, row)))
        .collect::<BTreeMap<_, _>>();
    if legacy && !s.retail.linked_files().is_empty() {
        let categories = s
            .retail
            .legacy_table("schedule_work_night_category_list.nei")?;
        labels.extend(
            (1..categories.rows)
                .filter(|&row| categories.exists(0, row))
                .map(|row| (categories.int(0, row), categories.string(1, row))),
        );
    }

    let linked = legacy && s.retail.linked_files().exists("recollection_vip2.nei");
    let legacy_disabled = if linked {
        ids(s.retail.table("schedule_work_legacy_disable.nei")?.as_ref())
    } else {
        BTreeSet::new()
    };
    let mut rows = BTreeMap::new();
    for source in [false, true] {
        let (list, work) = if source {
            if !linked {
                continue;
            }
            (
                Arc::new(s.retail.legacy_table("recollection_vip2.nei")?),
                Arc::new(s.retail.legacy_table("schedule_work_night.nei")?),
            )
        } else {
            if legacy {
                continue;
            }
            (
                s.retail.table("recollection_vip2.nei")?,
                s.retail.table("schedule_work_night.nei")?,
            )
        };
        for row in 1..list.rows {
            let id = list.int(1, row);
            if !list.exists(0, row) || ntr.contains(&id) || legacy_disabled.contains(&id) {
                continue;
            }
            let Ok(w) = row_id(&work, id) else { continue };
            let mode_column = if source { 5 } else { 6 };
            let category = work.int(4, w);
            let mode = work.string(mode_column, w);
            let Some(state) = s.player.night_works_state.get(&id) else {
                continue;
            };
            let role = work.string(if source { 31 } else { 32 }, w);
            if !matches!(mode.as_str(), "Vip" | "VipCall")
                || legacy && matches!(category, 51 | 52)
                || role != sub.as_deref().unwrap_or_default()
            {
                continue;
            }
            if !source
                && work
                    .string(33, w)
                    .split('&')
                    .filter(|v| !v.is_empty())
                    .any(|flag| s.player.flag(flag) < 1)
            {
                continue;
            }
            let reception = work.string(if source { 34 } else { 36 }, w);
            if !reception.is_empty() && !reception.split('|').any(|key| key == personal)
                || !main && work.string(if source { 35 } else { 37 }, w) == "○"
            {
                continue;
            }
            let first = mode_column + 6;
            let conditions = (first..first + 9)
                .filter(|&col| work.exists(col, w))
                .map(|col| work.string(col, w))
                .collect();
            rows.entry(list.int(0, row)).or_insert_with(|| Vip {
                work: id,
                category,
                icon: work.string(3, w),
                conditions,
                replay: Replay {
                    title: work.string(1, w),
                    file: list.string(2, row),
                    description: work.string(mode_column + 1, w),
                    enabled: state.finish,
                },
            });
        }
    }
    Ok((labels, rows.into_values().collect()))
}
