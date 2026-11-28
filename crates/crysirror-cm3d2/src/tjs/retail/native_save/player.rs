// SPDX-License-Identifier: MPL-2.0
use super::super::{TjsState, daily::SlotBackup};
use crate::{
    binio::{OrderedMap, OrderedSet},
    params::ScheduleData,
    save::player_1040::{Assignment, Player, Slot},
};
use anyhow::{Result, ensure};

const SLOTS: usize = 40;

pub(crate) fn capture_player(state: &TjsState) -> Result<Player> {
    let p = &state.player;
    let retail = &state.retail;
    ensure!(
        p.schedule_slot.len() == SLOTS,
        "invalid retail schedule length"
    );
    let slots = p
        .schedule_slot
        .iter()
        .enumerate()
        .map(|(i, slot)| Slot {
            guid: slot.maid_guid.clone(),
            outcome: [slot.noon_success_level, slot.night_success_level],
            communication: retail
                .slot_communication
                .get(&i)
                .copied()
                .unwrap_or([false; 2]),
            backup_status: pairs(&slot.backup_status),
        })
        .collect();
    // ScheduleSlotBackup.Type stores the predecessor bank first and
    // the standalone bank second regardless of the current mode.
    let backups = [&retail.legacy_schedule_backup, &retail.schedule_backup]
        .into_iter()
        .map(|bank| {
            (0..SLOTS)
                .map(|i| {
                    bank.get(i)
                        .map_or_else(Assignment::default, |row| Assignment {
                            guid: row.guid.clone(),
                            work: row.work,
                            communication: row.communication,
                        })
                })
                .collect()
        })
        .collect();
    Ok(Player {
        name: p.player_name.clone(),
        days: p.days,
        total_purchases: p.shop_use_money,
        money: p.money,
        coins: retail.casino_coins,
        club_name: retail
            .club_name
            .clone()
            .unwrap_or_else(|| "エンパイアクラブ".into()),
        grade: p.current_salon_grade,
        evaluation: p.salon_evaluation,
        gauge: state.club_gauge,
        flags: pairs(&p.generic_flag),
        parts: retail
            .owned_parts
            .iter()
            .map(|(key, value)| (key.clone(), *value))
            .collect(),
        items: pairs(&p.have_item),
        shop: pairs(&p.shop_lineup),
        slots,
        backups,
        night_jobs: pairs(&p.night_works_state),
        trophies: p.have_trophy.iter().map(|(id, ())| *id).collect(),
        job_classes: p.maid_class_open.iter().map(|(id, ())| *id).collect(),
        night_classes: p.yotogi_class_open.iter().map(|(id, ())| *id).collect(),
    })
}

pub(crate) fn restore_player(state: &mut TjsState, saved: &Player) -> Result<()> {
    ensure!(
        saved.slots.len() == SLOTS
            && saved.backups.len() == 2
            && saved.backups.iter().all(|bank| bank.len() == SLOTS),
        "1.0.4 requires 40 schedule slots and two backup banks",
    );
    let p = &mut state.player;
    let retail = &mut state.retail;
    p.player_name.clone_from(&saved.name);
    p.days = saved.days;
    p.shop_use_money = saved.total_purchases;
    p.money = saved.money;
    p.current_salon_grade = saved.grade;
    p.salon_evaluation = saved.evaluation;
    p.generic_flag = ordered(&saved.flags);
    p.have_item = ordered(&saved.items);
    p.shop_lineup = ordered(&saved.shop);
    p.night_works_state = ordered(&saved.night_jobs);
    p.have_trophy = set(&saved.trophies);
    p.maid_class_open = set(&saved.job_classes);
    p.yotogi_class_open = set(&saved.night_classes);
    p.schedule_slot = saved
        .slots
        .iter()
        .map(|slot| ScheduleData {
            maid_guid: slot.guid.clone(),
            noon_success_level: slot.outcome[0],
            night_success_level: slot.outcome[1],
            communication: slot.communication[0],
            backup_status: ordered(&slot.backup_status),
        })
        .collect();
    retail.slot_communication = saved
        .slots
        .iter()
        .map(|slot| slot.communication)
        .enumerate()
        .collect();
    let restore_bank = |bank: &[Assignment]| {
        bank.iter()
            .map(|row| SlotBackup {
                guid: row.guid.clone(),
                work: row.work,
                communication: row.communication,
            })
            .collect()
    };
    retail.legacy_schedule_backup = restore_bank(&saved.backups[0]);
    retail.schedule_backup = restore_bank(&saved.backups[1]);
    retail.casino_coins = saved.coins;
    retail.club_name = Some(saved.club_name.clone());
    retail.owned_parts = saved.parts.iter().cloned().collect();
    state.club_gauge = saved.gauge;
    Ok(())
}

pub(super) fn pairs<K: Clone + PartialEq, V: Clone>(map: &OrderedMap<K, V>) -> Vec<(K, V)> {
    map.iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

pub(super) fn ordered<K: Clone + PartialEq, V: Clone>(pairs: &[(K, V)]) -> OrderedMap<K, V> {
    let mut result = OrderedMap::default();
    for (key, value) in pairs {
        result.insert(key.clone(), value.clone());
    }
    result
}

pub(super) fn set<T: Clone + PartialEq>(values: &[T]) -> OrderedSet<T> {
    let mut result = OrderedSet::default();
    for value in values {
        result.add(value.clone());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_management_state_round_trip_retains_both_schedule_banks() -> Result<()> {
        let mut saved = Player {
            name: "支配人".into(),
            days: 8,
            total_purchases: 1_750_000,
            money: 9_876_543_210,
            coins: 123_456,
            club_name: "試験".into(),
            grade: 2,
            evaluation: 15,
            gauge: 778,
            slots: vec![Slot::default(); SLOTS],
            backups: vec![vec![Assignment::default(); SLOTS]; 2],
            ..Default::default()
        };
        saved.flags.push(("時間帯".into(), 3));
        saved.parts.push(("example.menu".into(), true));
        saved.slots[39].guid = "fixture-worker".into();
        saved.slots[39].communication = [false, true];
        saved.slots[39].outcome = [2, 4];
        saved.slots[39]
            .backup_status
            .push(("Morning_work".into(), 47));
        saved.backups[0][3] = Assignment {
            guid: "legacy".into(),
            work: [11, 12],
            communication: [true, false],
        };
        saved.backups[1][39] = Assignment {
            guid: "fixture-worker".into(),
            work: [3000, 10_000],
            communication: [false, true],
        };

        let mut state = TjsState::default();
        restore_player(&mut state, &saved)?;
        assert_eq!(state.player.flag("時間帯"), 3);
        assert_eq!(state.retail.casino_coins, 123_456);
        assert_eq!(state.retail.slot_communication[&39], [false, true]);
        assert_eq!(capture_player(&state)?.encode()?, saved.encode()?);

        let before = capture_player(&state)?;
        saved.backups.pop();
        assert!(restore_player(&mut state, &saved).is_err());
        assert_eq!(capture_player(&state)?, before);
        Ok(())
    }
}
