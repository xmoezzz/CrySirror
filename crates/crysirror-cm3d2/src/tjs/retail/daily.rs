// SPDX-License-Identifier: MPL-2.0
//! Standalone ScheduleAPI.SlotEmptyCheck and the daytime manager's morning backups.
use super::*;

#[derive(Clone, Default)]
pub(super) struct SlotBackup {
    pub(super) guid: String,
    pub(super) work: [i32; 2],
    pub(super) communication: [bool; 2],
}

pub(super) fn fill_empty_roster(state: &mut TjsState) -> Result<()> {
    ensure!(
        state.player.schedule_slot.len() == 40,
        "invalid standalone schedule length"
    );
    if (0..40).any(|i| state.schedule_slot(i).is_some()) {
        return Ok(());
    }
    let mut stock = state.stock.iter().filter_map(|i| state.maids.get(i));
    if let Some(maid) = stock.clone().find(|m| m.leader).or_else(|| stock.next()) {
        state.player.schedule_slot[0]
            .maid_guid
            .clone_from(&maid.guid_id);
    }
    Ok(())
}

pub(crate) fn close_daily(state: &mut TjsState, day: bool) -> Result<()> {
    let facilities = facility::daily_assignments(state)?;
    if !day {
        if let Some(refreshed) = facilities {
            state.retail.facilities = refreshed;
        }
        return Ok(());
    }
    ensure!(
        state.player.schedule_slot.len() == 40,
        "invalid standalone schedule length"
    );
    let legacy = state.player.flag("2.0モード") == 1;
    let mut slots = if legacy {
        state.retail.legacy_schedule_backup.clone()
    } else {
        state.retail.schedule_backup.clone()
    };
    slots.resize_with(40, Default::default);
    let mut writes = Vec::new();
    for (i, slot) in slots.iter_mut().enumerate() {
        let Some(id) = state.schedule_slot(i) else {
            slot.guid.clear();
            continue;
        };
        let values = snapshot(state, id)?;
        let maid = &state.maids[&id];
        slot.guid.clone_from(&maid.guid_id);
        slot.work = [maid.noon_work_id, maid.night_work_id];
        slot.communication = state.retail.characters.get(&maid.guid_id).map_or([false; 2], |c| c.communication);
        writes.push((i, values));
    }
    for (i, values) in writes {
        for (key, value) in values {
            state.player.schedule_slot[i]
                .backup_status
                .insert(format!("Morning_{key}"), value);
        }
    }
    if legacy {
        state.retail.legacy_schedule_backup = slots;
    } else {
        state.retail.schedule_backup = slots;
    }
    if let Some(refreshed) = facilities {
        state.retail.facilities = refreshed;
    }
    Ok(())
}

fn snapshot(state: &TjsState, id: usize) -> Result<[(&'static str, i32); 32]> {
    let maid = &state.maids[&id];
    let blank = character::Status::default();
    let c = state.retail.characters.get(&maid.guid_id).unwrap_or(&blank);
    let class = |kind: usize, id: i32, levels: &BTreeMap<i32, i32>| -> Result<(i32, i32)> {
        let Some(&level) = levels.get(&id) else {
            return Ok((0, 0));
        };
        Ok((
            level,
            *c.class_exp[kind]
                .get(&id)
                .context("missing standalone class experience")?,
        ))
    };
    let (job, job_exp) = class(0, maid.current_maid_class, &c.job_levels)?;
    let (skill, skill_exp) = class(1, maid.current_yotogi_class, &c.skill_class_levels)?;
    let p = &state.player;
    let param = |name| character::parameter(maid, c, name);
    Ok([
        ("clubMoney", p.money as i32),
        ("clubEvaluation", p.salon_evaluation),
        ("salon_cleanliness", 0),
        ("salon_gorgeous", 0),
        ("salon_evaluation", p.salon_evaluation),
        ("money_top0", (p.money % 1_000_000_000) as i32),
        ("money_top10", (p.money / 1_000_000_000) as i32),
        ("evaluation", maid.evaluation as i32),
        ("sales_top0", (maid.sales % 1_000_000_000) as i32),
        ("sales_top10", (maid.sales / 1_000_000_000) as i32),
        ("study_rate", maid.study_rate),
        ("reception", param("接待")?),
        ("care", param("お世話")?),
        ("lovely", param("可憐")?),
        ("elegance", param("気品")?),
        ("charm", param("魅惑")?),
        ("teach_rate", param("指導")?),
        ("cooking", param("料理")?),
        ("vocal", param("ボーカル")?),
        ("dance", param("ダンス")?),
        ("appealPoint", param("アピールポイント")?),
        ("inyoku", param("淫欲")?),
        ("m_value", param("M性")?),
        ("hentai", param("変態")?),
        ("housi", param("奉仕")?),
        ("maidClassLv", job),
        ("maidClassExp", job_exp),
        ("yotogiClassLv", skill),
        ("yotogiClassExp", skill_exp),
        ("yotogi_play_count", maid.yotogi_play_count),
        ("others_play_count", maid.others_play_count),
        ("likability", maid.likability),
    ])
}

pub(super) fn backup(state: &mut TjsState, slot: usize, phase: &str) -> Result<()> {
    if let Some(id) = state.schedule_slot(slot) {
        for (key, value) in snapshot(state, id)? {
            state.player.schedule_slot[slot].backup_status.insert(format!("{phase}_{key}"), value);
        }
    }
    Ok(())
}

pub(super) fn restore(state: &mut TjsState) {
    let bank = if state.player.flag("2.0モード") == 1 {
        &state.retail.legacy_schedule_backup
    } else {
        &state.retail.schedule_backup
    };
    for (slot, target) in state.player.schedule_slot.iter_mut().enumerate() {
        target.maid_guid.clear();
        let Some(saved) = bank.get(slot) else { continue; };
        let Some(id) = state.stock.iter().copied().find(|id| state.maids.get(id).is_some_and(|m| m.guid_id == saved.guid)) else { continue; };
        let m = state.maids.get_mut(&id).unwrap();
        target.maid_guid.clone_from(&m.guid_id);
        [m.noon_work_id, m.night_work_id] = saved.work;
        state.retail.characters.entry(m.guid_id.clone()).or_default().communication = saved.communication;
        state.retail.slot_communication.insert(slot, saved.communication);
    }
}

#[test]
fn daily_leader_and_morning_backups_are_transactional() -> Result<()> {
    let mut vm = super::tests::script()?;
    let s = &mut vm.runtime.host_mut().state;
    s.maids.get_mut(&1).unwrap().leader = true;
    fill_empty_roster(s)?;
    assert_eq!(s.player.schedule_slot[0].maid_guid, "character-1");
    s.player.schedule_slot[39].maid_guid = "character-0".into();
    s.player.money = 5_000_000_008;
    let m = s.maids.get_mut(&0).unwrap();
    m.noon_work_id = 120;
    m.night_work_id = 180;
    m.sales = 3_000_000_009;
    m.current_maid_class = 10;
    let c = s.retail.characters.entry(m.guid_id.clone()).or_default();
    c.communication = [true, false];
    c.job_levels.insert(10, 3);
    assert!(close_daily(s, true).is_err());
    assert!(
        s.player.schedule_slot[0]
            .backup_status
            .get("Morning_clubMoney")
            .is_none()
    );
    s.retail
        .characters
        .get_mut("character-0")
        .unwrap()
        .class_exp[0]
        .insert(10, 47);
    close_daily(s, true)?;
    let d = &s.player.schedule_slot[39].backup_status;
    for (key, value) in [
        ("money_top10", 5),
        ("money_top0", 8),
        ("sales_top10", 3),
        ("sales_top0", 9),
        ("maidClassLv", 3),
        ("maidClassExp", 47),
        ("clubMoney", 5_000_000_008i64 as i32),
    ] {
        assert_eq!(d.get(format!("Morning_{key}").as_str()), Some(&value));
    }
    assert_eq!(d.iter().count(), 32);
    assert_eq!(s.retail.schedule_backup[39].work, [120, 180]);
    assert_eq!(s.retail.schedule_backup[39].communication, [true, false]);
    s.player.schedule_slot[39].maid_guid.clear();
    close_daily(s, false)?;
    assert_eq!(s.retail.schedule_backup[39].guid, "character-0");
    close_daily(s, true)?;
    assert!(s.retail.schedule_backup[39].guid.is_empty());
    assert_eq!(s.retail.schedule_backup[39].work, [120, 180]);
    Ok(())
}
