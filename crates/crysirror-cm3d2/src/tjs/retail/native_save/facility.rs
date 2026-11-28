// SPDX-License-Identifier: MPL-2.0
use super::super::{
    State, TjsState, construction, facility::Facility, operations, row_id, wardrobe,
};
use crate::{
    params::Exp,
    save::facility_v5::{Facilities, Room},
};
use anyhow::{Context, Result, ensure};
use std::collections::BTreeMap;

// FacilityMgr uses a persistent string-keyed counter table.
const UPGRADES: &str = "施設強化回数";

pub(crate) fn restore_facilities(state: &mut TjsState, saved: &Facilities) -> Result<()> {
    let retail = &mut state.retail;
    let defs = retail.table("facility.nei")?;
    let restore_room = |slot: i32, room: &Room| -> Result<Facility> {
        row_id(&defs, room.type_id)?;
        ensure!(
            (0..construction::SLOTS as i32).contains(&slot),
            "invalid facility slot"
        );
        let mode = match room.costume_type {
            0 => wardrobe::Mode::Default,
            1 => wardrobe::Mode::Edited,
            2 => wardrobe::Mode::Heroine,
            _ => anyhow::bail!("invalid facility costume mode"),
        };
        let mut costume = wardrobe::Wardrobe {
            mode,
            edits: Some(BTreeMap::new()),
            guid: room.guid.clone(),
            ..Default::default()
        };
        for (key, enabled, name) in &room.costume_parts {
            let key = super::super::character::mpn_name(key)?;
            costume
                .edits
                .as_mut()
                .unwrap()
                .insert(key.into(), (!name.is_empty()).then(|| name.clone()));
            if !enabled {
                costume.unforced.insert(key.into());
            }
        }
        Ok(Facility {
            type_id: room.type_id,
            slot: Some(usize::try_from(slot)?),
            name: Some(room.name.clone()),
            assigned: [
                room.day.iter().cloned().collect(),
                room.night.iter().cloned().collect(),
            ],
            before_assignment: [
                room.previous_day.iter().cloned().collect(),
                room.previous_night.iter().cloned().collect(),
            ],
            costume,
            recipes: room.recipes.iter().cloned().collect(),
            native_parameters: Some(room.parameters),
        })
    };
    let facilities = saved
        .rooms
        .iter()
        .map(|(i, room)| restore_room(*i, room))
        .collect::<Result<Vec<_>>>()?;
    let mut orders = construction::Orders::default();
    for (index, room) in &saved.pending {
        let plan = if let Some(room) = room {
            let restored = restore_room(*index, room)?;
            Some(construction::Planned {
                id: restored.type_id,
                name: restored.name,
                wardrobe: restored.costume,
            })
        } else {
            None
        };
        orders.insert(usize::try_from(*index)?, plan);
    }
    let levels = retail.table("facility_level_table.nei")?;
    let experience = saved
        .experience
        .iter()
        .map(|(id, value)| {
            let mut value = value.clone();
            value.need = facility_needs(&levels, *id);
            (*id, value)
        })
        .collect();
    let upgrade_count = saved
        .achievements
        .iter()
        .find(|(key, _)| key == UPGRADES)
        .map(|(_, value)| value.parse())
        .transpose()
        .context("invalid facility upgrade count")?
        .unwrap_or(0);

    retail.facilities = facilities;
    retail.construction = orders;
    retail.facility_exp = experience;
    retail.facility_build_counts = saved.build_counts.iter().copied().collect();
    retail.buildings = Some(
        saved
            .build_counts
            .iter()
            .filter_map(|&(id, n)| (n > 0).then_some(id))
            .collect(),
    );
    retail.peak_facilities = retail.facilities.len();
    retail.materials = saved
        .materials
        .iter()
        .filter_map(|&(id, owned)| owned.then_some(id))
        .collect();
    retail.upgrade_count = upgrade_count;
    retail.upgrades.clear();
    retail.native_facility_records = Some(saved.clone());
    Ok(())
}

pub(crate) fn capture_facilities(state: &mut TjsState) -> Result<Facilities> {
    let templates = state.retail.native_facility_records.clone();
    let facilities = state.retail.facilities.clone();
    let orders = state.retail.construction.clone();
    let mut rooms = Vec::new();
    for (index, room) in facilities.iter().enumerate() {
        let slot = i32::try_from(room.slot.unwrap_or(index))?;
        let saved = templates
            .as_ref()
            .and_then(|old| old.rooms.iter().find(|(i, _)| *i == slot))
            .map(|(_, room)| room);
        rooms.push((slot, capture_room(state, room, saved)?));
    }
    let mut pending = Vec::new();
    for (index, plan) in orders.iter() {
        let slot = i32::try_from(*index)?;
        let data = if let Some(plan) = plan {
            let saved = templates
                .as_ref()
                .and_then(|old| old.pending.iter().find(|(i, _)| *i == slot))
                .and_then(|(_, room)| room.as_ref());
            let room = Facility {
                type_id: plan.id,
                name: plan.name.clone(),
                costume: plan.wardrobe.clone(),
                ..Default::default()
            };
            Some(capture_room(state, &room, saved)?)
        } else {
            None
        };
        pending.push((slot, data));
    }
    let retail = &mut state.retail;
    let defs = retail.table("facility.nei")?;
    let levels = retail.table("facility_level_table.nei")?;
    let materials = retail.table("facility_material.nei")?;
    let materials = (1..materials.rows)
        .take_while(|&r| materials.exists(0, r))
        .map(|r| {
            let id = materials.int(0, r);
            (id, retail.materials.contains(&id))
        })
        .collect();
    let experience = (1..defs.rows)
        .take_while(|&r| defs.exists(0, r))
        .map(|r| {
            let id = defs.int(0, r);
            (
                id,
                retail.facility_exp.get(&id).cloned().unwrap_or_else(|| {
                    let mut exp = Exp::default();
                    exp.set_list(facility_needs(&levels, id));
                    exp
                }),
            )
        })
        .collect();
    let mut achievements = templates.map(|old| old.achievements).unwrap_or_default();
    if let Some((_, value)) = achievements.iter_mut().find(|(key, _)| key == UPGRADES) {
        *value = retail.upgrade_count.to_string();
    } else if retail.upgrade_count != 0 {
        achievements.push((UPGRADES.into(), retail.upgrade_count.to_string()));
    }
    Ok(Facilities {
        rooms,
        materials,
        pending,
        experience,
        build_counts: retail
            .facility_build_counts
            .iter()
            .map(|(&id, &count)| (id, count))
            .collect(),
        achievements,
    })
}

fn facility_needs(table: &crate::nei::Table, id: i32) -> Vec<i32> {
    let row = (1..table.rows).find(|&r| table.int(0, r) == id);
    (0..5)
        .map(|i| {
            row.filter(|&r| table.exists(i + 2, r))
                .map_or(i as i32 * 100, |r| table.int(i + 2, r))
        })
        .collect()
}

fn capture_room(state: &mut TjsState, room: &Facility, template: Option<&Room>) -> Result<Room> {
    let template = template.filter(|template| template.guid == room.costume.guid);
    let defs = state.retail.table("facility.nei")?;
    let row = row_id(&defs, room.type_id)?;
    let defaults = template.filter(|_| room.recipes.is_empty());
    let recipes = if let Some(template) = defaults {
        template.recipes.clone()
    } else {
        operations::upgrade::recipes_for_type(state, room.type_id)?
            .into_iter()
            .map(|recipe| {
                let materials = room
                    .recipes
                    .get(&recipe.id)
                    .cloned()
                    .unwrap_or_else(|| vec![0; recipe.categories.len()]);
                (recipe.id, materials)
            })
            .collect()
    };
    let costume_type = match room.costume.mode {
        wardrobe::Mode::Default => 0,
        wardrobe::Mode::Edited => 1,
        wardrobe::Mode::Heroine => 2,
    };
    let costume_parts = capture_wardrobe(&mut state.retail, room)?;
    ensure!(!room.costume.guid.is_empty(), "missing native room GUID");
    Ok(Room {
        guid: room.costume.guid.clone(),
        type_id: room.type_id,
        name: room.name.clone().unwrap_or_else(|| defs.string(2, row)),
        parameters: room
            .native_parameters
            .or_else(|| template.map(|old| old.parameters))
            .unwrap_or_else(|| std::array::from_fn(|i| defs.int(i + 11, row))),
        recipes,
        costume_type,
        costume_parts,
        day: room.assigned[0].iter().cloned().collect(),
        night: room.assigned[1].iter().cloned().collect(),
        previous_day: room.before_assignment[0].iter().cloned().collect(),
        previous_night: room.before_assignment[1].iter().cloned().collect(),
    })
}

fn capture_wardrobe(state: &mut State, room: &Facility) -> Result<Vec<(String, bool, String)>> {
    let defaults;
    let items = if let Some(edits) = &room.costume.edits {
        edits
    } else {
        defaults = wardrobe::defaults(state, room.type_id, false)?.0;
        &defaults
    };
    Ok(wardrobe::enabled(state)?
        .into_iter()
        .map(|name| {
            let enabled = items.contains_key(name) && !room.costume.unforced.contains(name);
            let file = items
                .get(name)
                .and_then(|value| value.clone())
                .unwrap_or_default();
            (name.into(), enabled, file)
        })
        .collect())
}
