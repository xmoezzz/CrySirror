// SPDX-License-Identifier: MPL-2.0
//! The initial OnStartDay before the club has hired a character.
use super::*;
use crate::params::NightWorkState;

/// Resolve the entire first-day transaction before changing live player data.
/// An occupied roster requires the assignment, travel and result backup controllers.
pub(crate) fn start_before_hiring(s: &mut TjsState) -> Result<()> {
    ensure!(
        s.player.days == 0
            && s.stock.is_empty()
            && s.player
                .schedule_slot
                .iter()
                .all(|v| v.maid_guid.is_empty())
            && s.player.night_works_state.is_empty()
            && s.retail.facilities.is_empty()
            && !s.retail.compatibility
            && s.retail.legacy_files.is_empty(),
        "COM3D2 OnStartDay requires the populated schedule or linked-installation controller"
    );
    let facilities = s.retail.table("facility.nei")?;
    let fixed = (1..facilities.rows)
        .filter(|&r| facilities.exists(0, r) && facilities.string(15, r) == "〇")
        .map(|r| facilities.int(0, r))
        .collect::<BTreeSet<_>>();
    let ids = night_ids(&mut s.retail)?;
    let files = s.retail.source()?.files.clone();
    // DayStartManage calls OccurNightWork twice. Its global eligibility
    // check has no maid. VipCall needs an eligible stock maid. Easy jobs
    // override ordinary records with the same ID and add a trophy condition.
    let mut entries = crate::binio::OrderedMap::default();
    for (name, kind_column, easy) in [
        ("schedule_work_night.nei", 6, false),
        ("schedule_work_easyyotogi.nei", 4, true),
    ] {
        if !files.exists(name) {
            continue;
        }
        let table = s.retail.table(name)?;
        for r in 1..table.rows {
            let id = table.int(0, r);
            if id <= 0 || !ids.contains(&id) {
                continue;
            }
            if table.string(1, r).is_empty() {
                break;
            }
            let eligible = table.string(kind_column, r) == "Vip"
                && if easy {
                    s.player.have_trophy.contains(&table.int(7, r))
                } else {
                    table.int(21, r) <= s.player.current_salon_grade
                };
            entries.insert(id, eligible);
        }
    }
    let active = s.active_maids.iter().flatten().copied().next();
    if let Some(i) = active {
        ensure!(s.maids.contains_key(&i), "missing active maid status");
    }
    // FacilityManager.UpdateNextDay has no pending orders on a new game.
    s.retail.peak_facilities = fixed.len();
    s.retail.buildings = Some(fixed);
    for (&id, &enabled) in entries.iter() {
        if enabled {
            s.player.night_works_state.insert(
                id,
                NightWorkState {
                    work_id: id,
                    ..Default::default()
                },
            );
        }
    }
    s.player
        .generic_flag
        .insert("_Schedule_Noon_Resulted".into(), 0);
    s.player
        .generic_flag
        .insert("_Schedule_Night_Resulted".into(), 0);
    if let Some(maid) = active.and_then(|i| s.maids.get_mut(&i)) {
        maid.leader = true;
    }
    // Slot backups have no owners yet. Ranking is unavailable in this ABI.
    s.player.days += 1;
    Ok(())
}

pub(super) fn night_ids(s: &mut State) -> Result<BTreeSet<i32>> {
    // CVSRead merges enabled IDs from the base list, suffixed enabled lists
    // and plugin lists. The legacy list belongs to a separate filesystem.
    s.enabled("schedule_work_night_enabled", 0)?;
    let mut ids = s.enabled_lists["schedule_work_night_enabled"].clone();
    let files = s.source()?.files.clone();
    for suffix in s.suffixes.clone().unwrap_or_default() {
        let path = format!("schedule_work_night_{suffix}.nei");
        if suffix != "legacy" && files.exists(&path) {
            let t = s.table(&path)?;
            ids.extend((1..t.rows).filter(|&r| t.exists(0, r)).map(|r| t.int(0, r)));
        }
    }
    Ok(ids)
}
