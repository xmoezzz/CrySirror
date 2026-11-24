// SPDX-License-Identifier: MPL-2.0
use super::*;

#[derive(Clone, Default)]
pub(super) struct Facility {
    pub type_id: i32,
    pub assigned: [BTreeSet<String>; 2],
    pub costume: Costume,
}
#[derive(Clone, Default)]
pub(super) enum Costume {
    Default,
    Edited(BTreeMap<String, Option<String>>),
    #[default]
    Heroine,
}

pub(super) fn roster(s: &TjsState) -> Vec<usize> {
    s.player
        .schedule_slot
        .iter()
        .take(40)
        .filter(|slot| !slot.maid_guid.is_empty())
        .filter_map(|slot| {
            s.stock
                .iter()
                .copied()
                .find(|i| s.maids.get(i).is_some_and(|m| m.guid_id == slot.maid_guid))
        })
        .collect()
}
fn shift(s: &TjsState) -> usize {
    usize::from(s.player.flag("時間帯") == 3)
}
fn assignment<'a>(s: &'a State, guid: &str, shift: usize) -> Option<&'a Facility> {
    s.facilities
        .iter()
        .find(|f| f.assigned[shift].contains(guid))
}

fn communication_candidates(s: &mut TjsState, marked: bool) -> Result<Vec<(usize, Option<usize>)>> {
    let mut result = Vec::new();
    let shift = shift(s);
    let roster = roster(s);
    let work_id = |m: &crate::params::MaidStatus| {
        if shift == 0 {
            m.noon_work_id
        } else {
            m.night_work_id
        }
    };
    if roster.is_empty() {
        return Ok(result);
    }
    let training = s.retail.table("schedule_work_noon.nei")?;
    let facilities = s.retail.table("facility.nei")?;
    let mut types = BTreeMap::new();
    let mut companions = BTreeMap::new();
    for r in (1..training.rows).filter(|&r| training.exists(0, r)) {
        let id = training.int(0, r);
        if !s.retail.enabled("schedule_work_noon_enabled", id)? {
            continue;
        }
        if training.string(1, r).is_empty() {
            break;
        }
        let role = training.string(5, r);
        companions.entry(role.clone()).or_insert(id);
        types.insert(id, role);
    }
    let def = s.retail.table("schedule_define.nei")?;
    let power = (0..def.rows)
        .filter(|&r| def.string(0, r) == "faclilityPowerUpWorkId")
        .map(|r| def.int(1, r))
        .next_back()
        .unwrap_or(3000);
    for &i in &roster {
        let m = &s.maids[&i];
        if marked
            && !s
                .retail
                .characters
                .get(&m.guid_id)
                .is_some_and(|c| c.communication[shift])
        {
            continue;
        }
        let work = work_id(m);
        let training_role = types.get(&work);
        let role = if let Some(role) = training_role {
            role.clone()
        } else {
            let Some(f) = assignment(&s.retail, &m.guid_id, shift) else {
                continue;
            };
            let assigned = row_id(&facilities, f.type_id)?;
            if (f.assigned[shift].len() as i32) < facilities.int(20, assigned) {
                continue;
            }
            if work == power {
                String::new()
            } else {
                let Some(r) = (1..facilities.rows)
                    .find(|&r| facilities.exists(0, r) && facilities.int(1, r) == work)
                else {
                    continue;
                };
                facilities.string(25, r)
            }
        };
        // Unset or unrecognised enum text leaves TrainingType at its zero value (Basic).
        let role = match role.as_str() {
            "Trainer" | "Trainee" | "Holiday" | "Dispatch" | "Travel" => role.as_str(),
            _ => "Basic",
        };
        if role != "Basic" && !(marked && matches!(role, "Trainer" | "Trainee" | "Holiday")) {
            continue;
        }
        let opposite = match training_role.map(String::as_str) {
            Some("Trainer") => Some("Trainee"),
            Some("Trainee") => Some("Trainer"),
            _ => None,
        };
        let second = opposite.and_then(|r| {
            let other = companions.get(r).copied().unwrap_or(0);
            roster
                .iter()
                .copied()
                .find(|j| work_id(&s.maids[j]) == other)
        });
        result.push((i, second));
        if marked {
            break;
        }
    }
    Ok(result)
}

pub(super) fn communication(s: &mut TjsState, _: &[Variant]) -> Result<Variant> {
    let Some((first, companion)) = communication_candidates(s, true)?.into_iter().next() else {
        return Ok(boolean(false));
    };
    for (slot, id) in std::iter::once(first).chain(companion).enumerate() {
        activate(s, slot, id, true);
    }
    Ok(boolean(true))
}

pub(super) fn activate(s: &mut TjsState, slot: usize, id: usize, visible: bool) {
    for current in &mut s.active_maids {
        if *current == Some(id) {
            *current = None;
        }
    }
    crate::tjs::set_active(s, slot, id, visible);
}

pub(super) fn has_communication(s: &mut TjsState, _: &[Variant]) -> Result<Variant> {
    ensure!(
        s.retail.compatibility || s.player.flag("2.0モード") == 1,
        "IsCommunicationChara requires legacy mode"
    );
    Ok(boolean(!communication_candidates(s, false)?.is_empty()))
}

pub(super) fn random_communication(s: &mut TjsState, _: &[Variant]) -> Result<Variant> {
    s.retail.stop_motions = true;
    let list = communication_candidates(s, false)?;
    if list.is_empty() {
        if let Some(slot) = s.active_maids.first_mut() {
            *slot = None;
        }
        s.requests.push(crate::tjs::Request::DeactivateMaid(0));
    } else {
        let id = list[s.random(list.len())].0;
        activate(s, 0, id, true);
    }
    Ok(Variant::Void)
}

fn night_roles(s: &mut State) -> Result<BTreeMap<i32, String>> {
    let ids = day::night_ids(s)?;
    let mut roles = BTreeMap::new();
    for (name, column) in [
        ("schedule_work_night.nei", 6),
        ("schedule_work_easyyotogi.nei", 4),
    ] {
        if !s.source()?.files.exists(name) {
            continue;
        }
        let table = s.table(name)?;
        for r in 1..table.rows {
            let id = table.int(0, r);
            if !ids.contains(&id) {
                continue;
            }
            if table.string(1, r).is_empty() {
                break;
            }
            roles.insert(id, table.string(column, r));
        }
    }
    Ok(roles)
}

fn matching_work(
    s: &mut TjsState,
    night: bool,
    reception: bool,
    roles: &[&str],
) -> Result<Vec<usize>> {
    let roster = roster(s);
    if roster.is_empty() {
        return Ok(roster);
    }
    let catalog = night_roles(&mut s.retail)?;
    let mut output = Vec::new();
    for id in roster {
        let maid = &s.maids[&id];
        let work = if night {
            maid.night_work_id
        } else {
            maid.noon_work_id
        };
        if let Some(role) = catalog.get(&work) {
            if roles.contains(&role.as_str()) {
                output.push(id);
            }
        } else if reception {
            bail!("invalid reception night assignment {work}");
        }
    }
    Ok(output)
}

pub(super) fn select_work(s: &mut TjsState, roles: &[&str]) -> Result<Variant> {
    let list = matching_work(s, shift(s) == 1, false, roles)?;
    s.chara_guid_list = list.iter().map(|i| s.maids[i].guid_id.clone()).collect();
    Ok(Variant::Void)
}

pub(super) fn reception(s: &mut TjsState, random: bool) -> Result<Variant> {
    if random {
        s.retail.stop_motions = true;
    }
    let list = matching_work(s, true, true, &["Entertain"])?;
    if !random {
        return Ok(boolean(!list.is_empty()));
    }
    ensure!(!list.is_empty(), "no reception maid");
    let id = list[s.random(list.len())];
    activate(s, 0, id, true);
    Ok(Variant::Void)
}

fn reset(slot: &mut preset::Slot, base: i32) {
    if slot
        .temp
        .as_ref()
        .is_some_and(|(_, id)| *id != 0 && *id != slot.file_id)
        || slot.temp_value != base
    {
        slot.temp = None;
        slot.temp_value = 0;
        slot.dirty = true;
    }
}

pub(super) fn costume(s: &mut TjsState, a: &[Variant], reset_all: bool) -> Result<Variant> {
    arity(a, 1)?;
    let index = s
        .active_maids
        .get(arg_int(a, 0) as usize)
        .copied()
        .flatten()
        .context("empty costume slot")?;
    let guid = &s.maids.get(&index).context("missing maid")?.guid_id;
    let assigned = assignment(&s.retail, guid, shift(s));
    let (id, mode) = if reset_all {
        (0, Costume::Heroine)
    } else if let Some(f) = assigned {
        (f.type_id, f.costume.clone())
    } else {
        return Ok(Variant::Void);
    };
    let enabled = s.retail.table("facility_costume_mpn_enable_list.nei")?;
    let mut edits = BTreeMap::new();
    for r in (1..enabled.rows).take_while(|&r| enabled.exists(0, r)) {
        edits.insert(character::mpn_name(&enabled.string(0, r))?, None);
    }
    match mode {
        Costume::Heroine => {}
        Costume::Edited(values) => {
            edits.clear();
            for (name, file) in values {
                edits.insert(character::mpn_name(&name)?, file);
            }
        }
        Costume::Default => {
            let t = s.retail.table("facility_default_costume.nei")?;
            let Some(col) = (1..t.columns).find(|&col| t.int(col, 3) == id) else {
                return Ok(Variant::Void);
            };
            let enabled = edits;
            edits = BTreeMap::new();
            for r in (4..t.rows).take_while(|&r| t.exists(0, r)) {
                let Ok(name) = character::mpn_name(&t.string(0, r)) else {
                    break;
                };
                if !enabled.contains_key(name) {
                    continue;
                }
                let file = t.string(col, r);
                let file = if file.is_empty() {
                    None
                } else if file.eq_ignore_ascii_case("del") {
                    Some(preset::delete_menu(name).unwrap_or("").into())
                } else {
                    Some(format!("{file}.menu"))
                };
                edits.insert(name, file);
            }
        }
    }
    let appearance = s.retail.appearances.entry(index).or_default();
    for (name, file) in edits {
        let slot = appearance.slots.entry(name).or_default();
        let base = s
            .bodies
            .get(&index)
            .and_then(|b| b.props.get(name))
            .map_or(0, |v| v.1);
        let Some(file) = file else {
            reset(slot, base);
            continue;
        };
        let file = if file == "del" {
            let Some(del) = preset::delete_menu(name) else {
                continue;
            };
            del.into()
        } else {
            file
        };
        if slot.temp.as_ref().map_or("", |v| v.0.as_str()) != file {
            let id = crate::save::mono_string_hash(&file.to_lowercase());
            slot.temp = Some((file, id));
            slot.dirty = true;
        }
    }
    Ok(Variant::Void)
}

pub(super) fn built(s: &mut TjsState, t: &Table, id: i32) -> bool {
    let defaults = s.retail.buildings.get_or_insert_with(|| {
        (1..t.rows)
            .filter(|&r| t.exists(0, r) && t.string(15, r) == "〇")
            .map(|r| t.int(0, r))
            .collect()
    });
    defaults.contains(&id) || s.retail.facilities.iter().any(|f| f.type_id == id)
}
