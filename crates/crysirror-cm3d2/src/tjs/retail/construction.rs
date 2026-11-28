// SPDX-License-Identifier: MPL-2.0
//! Standalone SceneFacilityManagement reservations and FacilityManager.UpdateNextDayFacility.
use super::*;
use crate::binio::OrderedMap;

pub(crate) const SLOTS: usize = 12;
pub(crate) type Orders = OrderedMap<usize, Option<Planned>>;
#[derive(Clone)]
pub(crate) struct Planned {
    pub id: i32,
    pub(super) name: Option<String>,
    pub wardrobe: wardrobe::Wardrobe,
}

pub(crate) struct Definition {
    pub id: i32,
    pub name: String,
    pub thumbnail: String,
    pub cost: i64,
    pub rank: usize,
    pub minimum: i32,
    pub maximum: i32,
    fixed: bool,
    pub removable: bool,
    mode: String,
}

pub(crate) struct Catalog(Vec<Definition>);
impl Catalog {
    pub fn load(s: &mut State) -> Result<Self> {
        let t = s.table("facility.nei")?;
        Ok(Self(
            (1..t.rows)
                .take_while(|&r| t.exists(0, r))
                .map(|r| Definition {
                    id: t.int(0, r),
                    name: t.string(2, r),
                    thumbnail: t.string(18, r),
                    cost: i64::from(t.int(10, r)),
                    rank: t.int(8, r).max(0) as usize,
                    minimum: t.int(20, r),
                    maximum: t.int(21, r),
                    fixed: t.string(15, r) == "〇",
                    removable: t.string(16, r) == "〇",
                    mode: t.string(17, r),
                })
                .collect(),
        ))
    }

    pub fn get(&self, id: i32) -> Result<&Definition> {
        self.0
            .iter()
            .find(|v| v.id == id)
            .with_context(|| format!("missing facility definition {id}"))
    }
    pub fn live(s: &State, slot: usize) -> Option<i32> {
        s.facilities
            .iter()
            .enumerate()
            .find(|(i, f)| f.slot.unwrap_or(*i) == slot)
            .map(|(_, f)| f.type_id)
    }
    pub fn effective(s: &State, slot: usize) -> Option<i32> {
        s.construction
            .get(&slot)
            .map(|v| v.as_ref().map(|p| p.id))
            .unwrap_or_else(|| Self::live(s, slot))
    }
    pub fn display_name<'a>(&'a self, s: &'a State, slot: usize) -> Result<&'a str> {
        if let Some(Some(plan)) = s.construction.get(&slot) {
            return Ok(plan.name.as_deref().unwrap_or(&self.get(plan.id)?.name));
        }
        if let Some(i) = s.facility_index(slot) {
            let f = &s.facilities[i];
            return Ok(f.name.as_deref().unwrap_or(&self.get(f.type_id)?.name));
        }
        Ok("空き部屋")
    }
    pub fn rename(s: &mut State, slot: usize, value: &str) -> Result<()> {
        let value = value.trim();
        if value.is_empty() {
            return Ok(());
        }
        if let Some(Some(plan)) = s.construction.get_mut(&slot) {
            plan.name = Some(value.into());
        } else {
            let i = s
                .facility_index(slot)
                .context("cannot rename an empty facility")?;
            s.facilities[i].name = Some(value.into());
        }
        Ok(())
    }
    pub fn available<'a>(&'a self, s: &TjsState) -> impl Iterator<Item = &'a Definition> {
        let locked = s.player.flag("__lockNTRPlay") != 0;
        self.0.iter().filter(move |f| {
            !f.fixed
                && match f.mode.as_str() {
                    "" => true,
                    "×" => !locked,
                    "〇" | "○" => locked,
                    _ => !locked,
                }
        })
    }
    pub fn level(s: &State, id: i32) -> i32 {
        s.facility_exp.get(&id).map_or(1, |exp| exp.level)
    }
    pub fn choices(&self, s: &TjsState, slot: usize) -> Vec<&Definition> {
        let current = Self::effective(&s.retail, slot);
        self.available(s)
            .filter(|f| {
                Some(f.id) != current
                    && (!matches!(f.name.as_str(), "カジノ" | "高級カジノ")
                        || !(s.retail.facilities.iter().any(|r| r.type_id == f.id)
                            || s.retail
                                .construction
                                .iter()
                                .any(|(_, v)| v.as_ref().is_some_and(|p| p.id == f.id))))
            })
            .collect()
    }
    pub fn total(&self, s: &State, replacement: Option<(usize, i32)>) -> Result<i64> {
        let mut sum = 0;
        for (&slot, order) in s.construction.iter() {
            if replacement.is_some_and(|(i, _)| i == slot) {
                continue;
            }
            if let Some(plan) = order {
                sum += self.get(plan.id)?.cost;
            }
        }
        if let Some((_, id)) = replacement {
            sum += self.get(id)?.cost;
        }
        Ok(sum)
    }
    pub fn reserve(&self, s: &mut TjsState, slot: usize, id: Option<i32>) -> Result<()> {
        ensure!(slot < SLOTS, "invalid facility slot");
        if let Some(current) = Self::effective(&s.retail, slot) {
            ensure!(self.get(current)?.removable, "fixed facility");
        }
        if let Some(id) = id {
            ensure!(
                self.choices(s, slot).iter().any(|v| v.id == id),
                "unavailable facility"
            );
            ensure!(
                self.total(&s.retail, Some((slot, id)))? <= s.player.money,
                "insufficient construction funds"
            );
        } else {
            ensure!(
                Self::effective(&s.retail, slot).is_some(),
                "empty facility slot"
            );
        }
        let plan = id.map(|id| Planned {
            id,
            name: None,
            wardrobe: wardrobe::Wardrobe {
                guid: s.guid(),
                ..Default::default()
            },
        });
        s.retail.construction.insert(
            slot,
            plan,
        );
        Ok(())
    }
}

pub(super) fn advance(s: &mut TjsState) -> Result<()> {
    if s.retail.construction.is_empty() {
        return Ok(());
    }
    let catalog = Catalog::load(&mut s.retail)?;
    for (i, f) in s.retail.facilities.iter_mut().enumerate() {
        f.slot.get_or_insert(i);
    }
    // Mono Dictionary retains insertion order when an existing reservation is replaced.
    for (&slot, order) in std::mem::take(&mut s.retail.construction).iter() {
        let cost = order
            .as_ref()
            .map(|plan| catalog.get(plan.id).map(|v| v.cost))
            .transpose()?
            .unwrap_or(0);
        if s.player.money < cost {
            continue;
        }
        s.player.money -= cost;
        s.retail.facilities.retain(|f| f.slot != Some(slot));
        if let Some(plan) = order {
            let id = plan.id;
            s.retail
                .buildings
                .get_or_insert_with(Default::default)
                .insert(id);
            let count = s.retail.facility_build_counts.entry(id).or_default();
            *count = count.saturating_add(1);
            s.retail.facilities.push(facility::Facility {
                type_id: id,
                name: plan.name.clone(),
                costume: plan.wardrobe.clone(),
                slot: Some(slot),
                ..Default::default()
            });
        }
    }
    s.retail.facilities.sort_by_key(|f| f.slot);
    s.retail.peak_facilities = s.retail.peak_facilities.max(s.retail.facilities.len());
    Ok(())
}
