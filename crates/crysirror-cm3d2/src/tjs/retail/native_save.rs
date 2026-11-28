// SPDX-License-Identifier: MPL-2.0
//! Projection between COM3D2 1.0.4 native slot records and retail runtime state.
mod employee;
mod facility;
mod player;

pub(crate) use employee::{capture_employees, restore_employees};
pub(crate) use facility::{capture_facilities, restore_facilities};
pub(crate) use player::{capture_player, restore_player};

use super::{DeskPlacement, State, TjsState, activities, preset, scenario, stock};
use crate::save::{
    management_1040::{DeskItem, Management, Script},
    slot_1040::{Casino, Dealer, Slot},
    stock_1040::{Man, Misc, Roster},
};
use anyhow::{Result, ensure};
use std::{borrow::Cow, collections::BTreeSet};

pub(crate) struct NativeMan {
    pub model: crate::chara::Maid,
    appearance: preset::Appearance,
    misc: Misc<'static>,
}

impl NativeMan {
    pub(crate) fn active_slot(&self) -> Option<usize> {
        usize::try_from(self.misc.active_slot).ok()
    }
}

const MEN: usize = 6;

pub(crate) fn stage(
    template: &TjsState,
    slot: &Slot<'_>,
    client: crysirror_core::state::ClientId,
) -> Result<(TjsState, Vec<NativeMan>)> {
    ensure!(
        !slot.has_transferred_characters
            && !slot
                .roster
                .employees
                .iter()
                .any(|e| e.status.heroine == crate::save::status_1040::Heroine::Transfer),
        "transferred staff requires the linked CM3D2 save profile"
    );
    let installed = template.retail.source()?;
    for path in &slot.csv_paths[0] {
        ensure!(
            installed.files.csv_paths().contains(path),
            "missing save CSV archive {path}"
        );
    }
    // Clone installed resources and table caches before rebuilding everything
    // owned by the new game session.
    let mut state = TjsState::default();
    state.data.clone_from(&template.data);
    state.retail.source = Some(installed.clone());
    state.retail.tables = template.retail.tables.clone();
    state.retail.key = template.retail.key.clone();
    state.system_versions.clone_from(&template.system_versions);
    restore_player(&mut state, &slot.roster.player)?;
    for key in ["__lockNTRPlay", "__lockUserDraftMaid", "__isOldPlayer"] {
        state
            .player
            .generic_flag
            .insert(key.into(), template.player.flag(key));
    }
    restore_employees(&mut state, &slot.roster.employees, client)?;
    restore_facilities(&mut state, &slot.management.facilities)?;
    scenario::restore_slot(&mut state.retail, &slot.management.scenario)?;
    state.retail.desk = slot
        .management
        .desk
        .iter()
        .map(|item| {
            (
                item.key,
                DeskPlacement {
                    visible: item.visible,
                    season: item.seasonal,
                    transform: [item.position, item.rotation, item.scale],
                },
            )
        })
        .collect();
    restore_casino(&mut state, &slot.casino)?;
    state
        .retail
        .native_guest_slots
        .clone_from(&slot.guest_slots);
    let men = restore_men(&state, &slot.roster.men)?;
    state.player.generic_flag.insert("傅きモード".into(), 0);
    state.retail.adv_present = true;
    Ok((state, men))
}

pub(crate) fn capture<'a>(
    state: &mut TjsState,
    checkpoint: &'a [u8],
    fade_wait: bool,
    enabled: bool,
    men: &[NativeMan],
    comment: String,
) -> Result<Slot<'a>> {
    let source = state.retail.source()?.clone();
    let player = capture_player(state)?;
    let employees = capture_employees(state)?;
    let men = capture_men(state, men)?;
    let facilities = capture_facilities(state)?;
    let scenario = scenario::capture_slot(&mut state.retail)?;
    let desk = state
        .retail
        .desk
        .iter()
        .map(|(&id, p)| DeskItem {
            key: id,
            id,
            visible: p.visible,
            seasonal: p.season,
            position: p.transform[0],
            rotation: p.transform[1],
            scale: p.transform[2],
        })
        .collect();
    let casino = capture_casino(state)?;
    let guest_slots = state.retail.native_guest_slots.clone();
    Ok(Slot {
        header: crate::save::Header {
            save_time: chrono::Local::now().format("%Y%m%d%H%M%S").to_string(),
            game_day: player.days,
            player_name: player.name.clone(),
            maid_num: i32::try_from(employees.len())?,
            comment,
        },
        has_transferred_characters: employees
            .iter()
            .any(|e| e.status.heroine == crate::save::status_1040::Heroine::Transfer),
        csv_paths: [source.files.csv_paths().to_vec(), Vec::new()],
        roster: Roster {
            player,
            employees,
            men,
            script_offset: 0,
        },
        management: Management {
            script: Script {
                checkpoint,
                fade_wait,
                enabled,
            },
            desk,
            scenario,
            facilities,
        },
        guest_slots,
        casino,
    })
}

fn restore_men(state: &TjsState, men: &[Man<'_>]) -> Result<Vec<NativeMan>> {
    ensure!(men.len() == MEN, "standalone save requires six stock men");
    let source = state.retail.source()?;
    let mut active = BTreeSet::new();
    men.iter()
        .map(|man| {
            ensure!(
                man.misc.active_slot == -1
                    || ((0..MEN as i32).contains(&man.misc.active_slot)
                        && active.insert(man.misc.active_slot)),
                "duplicate or invalid active man slot",
            );
            let mut model = stock::retail_maid(true);
            model.set_up_model(&source.files, source.storage.as_ref(), "_I_man.menu")?;
            let appearance = preset::restore_model_props(&mut model, &man.properties, None)?;
            model.man_color = man.misc.color;
            model.all_proc_prop(&source.files, source.storage.as_ref())?;
            Ok(NativeMan {
                model,
                appearance,
                misc: man.misc.to_owned(),
            })
        })
        .collect()
}

fn capture_men(state: &TjsState, men: &[NativeMan]) -> Result<Vec<Man<'static>>> {
    let source = state.retail.source()?;
    if men.is_empty() {
        let mut model = stock::retail_maid(true);
        model.set_up_model(&source.files, source.storage.as_ref(), "_I_man.menu")?;
        return (0..MEN)
            .map(|i| {
                Ok(Man {
                    properties: preset::capture_model_props(&model, None)?,
                    misc: Misc {
                        active_slot: i32::try_from(i)?,
                        thumbnail: Cow::Borrowed(&[]),
                        thumbnail_time: String::new(),
                        color: model.man_color,
                        voice_pitch: 50,
                    },
                })
            })
            .collect();
    }
    ensure!(men.len() == MEN, "invalid runtime stock men count");
    men.iter()
        .map(|man| {
            Ok(Man {
                properties: preset::capture_model_props(&man.model, Some(&man.appearance))?,
                misc: man.misc.clone(),
            })
        })
        .collect()
}

fn casino_facilities(retail: &mut State) -> Result<[Option<usize>; 2]> {
    let defs = retail.table("facility.nei")?;
    Ok(["カジノ", "高級カジノ"].map(|name| {
        let row = (1..defs.rows).find(|&row| defs.exists(0, row) && defs.string(2, row) == name)?;
        let ty = defs.int(0, row);
        retail
            .facilities
            .iter()
            .enumerate()
            .filter(|(_, room)| room.type_id == ty)
            .map(|(i, room)| room.slot.unwrap_or(i))
            .min()
    }))
}

fn restore_casino(state: &mut TjsState, casino: &Casino) -> Result<()> {
    let locations = casino_facilities(&mut state.retail)?;
    for (location, dealer) in locations.into_iter().zip([&casino.low, &casino.high]) {
        ensure!(
            location.is_some() == dealer.is_some(),
            "casino slot does not match the facility state"
        );
        if let (Some(location), Some(dealer)) = (location, dealer) {
            let active = usize::try_from(dealer.active_index)
                .ok()
                .and_then(|i| state.active_maids.get(i))
                .copied()
                .flatten();
            let stock = usize::try_from(dealer.stock_index)
                .ok()
                .and_then(|i| state.stock.get(i))
                .copied();
            if let Some(id) = active.or(stock) {
                state.retail.casino_dealers.insert(location, id);
            }
        }
    }
    state.retail.casino_sold_out = casino
        .sold_out
        .iter()
        .filter_map(|&(id, sold)| sold.then_some(id))
        .collect();
    Ok(())
}

fn capture_casino(state: &mut TjsState) -> Result<Casino> {
    let locations = casino_facilities(&mut state.retail)?;
    let fallback = if locations.iter().any(Option::is_some) {
        activities::first_dealer(state).ok()
    } else {
        None
    };
    let list = state.retail.table("casinoshop_data.nei")?;
    let dealer = |location: Option<usize>| -> Result<Option<Dealer>> {
        let Some(location) = location else {
            return Ok(None);
        };
        let id = state
            .retail
            .casino_dealers
            .get(&location)
            .copied()
            .or(fallback);
        let active = id.and_then(|id| {
            state
                .active_maids
                .iter()
                .position(|&entry| entry == Some(id))
        });
        let stock = id
            .and_then(|id| state.stock.iter().position(|&entry| entry == id))
            .filter(|_| active.is_none());
        Ok(Some(Dealer {
            active_index: active.map(i32::try_from).transpose()?.unwrap_or(-1),
            stock_index: stock.map(i32::try_from).transpose()?.unwrap_or(-1),
        }))
    };
    let [low, high] = [dealer(locations[0])?, dealer(locations[1])?];
    let mut sold = Vec::new();
    let mut seen = BTreeSet::new();
    for row in (1..list.rows).filter(|&row| list.exists(0, row)) {
        let id = list.int(0, row);
        if seen.insert(id) && state.retail.enabled("casinoshop_enable_id", id)? {
            sold.push((id, state.retail.casino_sold_out.contains(&id)));
        }
    }
    Ok(Casino::from_parts(low, high, sold))
}
