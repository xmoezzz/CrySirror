// SPDX-License-Identifier: MPL-2.0
//! COM3D2 1.0.4 FreeSkillSelect catalogues and manager state.
use super::*;

pub(crate) const CATEGORIES: [&str; 9] = [
    "淫欲",
    "変態",
    "奉仕",
    "M性",
    "交換",
    "乱交",
    "詰られ",
    "ハーレム",
    "百合",
];
pub(crate) const CURRENT_TYPES: [&str; 6] = ["通常", "酔い", "目隠し", "媚薬", "告白", "気絶"];
pub(crate) const LEGACY_TYPES: [&str; 3] = ["通常", "お仕置き", "酔い"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Choice {
    pub skill: i32,
    pub level: u8,
    pub stage: String,
    pub day: bool,
}

pub(crate) struct Stage {
    pub key: String,
    pub name: String,
    pub backdrop: String,
    pub thumbnail: String,
    pub day: bool,
}

pub(crate) struct Skill {
    pub id: i32,
    pub name: String,
    pub kind: usize,
    pub category: usize,
    pub stages: BTreeSet<String>,
}

pub(crate) struct Catalogue {
    pub skills: Vec<Skill>,
    pub stages: Vec<Stage>,
    pub locked: bool,
    pub previous: Option<Choice>,
}

fn stages(s: &mut TjsState, legacy: bool) -> Result<Vec<Stage>> {
    if legacy {
        return Ok(linked::stages(s)?
            .into_iter()
            .filter(|s| s.enabled)
            .map(|s| Stage {
                key: s.key,
                name: s.name,
                backdrop: s.backdrop,
                thumbnail: s.thumbnail,
                day: false,
            })
            .collect());
    }
    let table = s.retail.table("yotogi_stage_list.nei")?;
    let mut stages = Vec::new();
    for r in 1..table.rows {
        if !table.exists(0, r)
            || !s
                .retail
                .enabled("yotogi_stage_enabled_list", table.int(0, r))?
            || s.player.current_salon_grade < table.int(5, r)
        {
            continue;
        }
        let facilities = table.string(6, r);
        if !facilities.trim().is_empty()
            && !facilities.split(',').any(|id| {
                id.trim().parse::<i32>().is_ok_and(|id| {
                    s.retail
                        .facility_build_counts
                        .get(&id)
                        .is_some_and(|&count| count > 0)
                })
            })
        {
            continue;
        }
        let day_backdrop = table.string(7, r);
        for (column, day) in [(7, true), (9, false)] {
            let backdrop = table.string(column, r);
            if backdrop.is_empty() || !day && backdrop == day_backdrop {
                continue;
            }
            stages.push((
                table.int(3, r),
                Stage {
                    key: table.string(1, r),
                    name: table.string(2, r),
                    backdrop,
                    thumbnail: table.string(column + 1, r),
                    day,
                },
            ));
        }
    }
    stages.sort_by_key(|(order, _)| *order);
    Ok(stages.into_iter().map(|(_, stage)| stage).collect())
}

fn class_enabled(state: &mut State, unique_name: &str, job: bool, legacy: bool) -> Result<bool> {
    let (table, list) = if job {
        (
            "maid_status_jobclass_list.nei",
            "maid_status_jobclass_enabled_list",
        )
    } else {
        (
            "maid_status_yotogiclass_list.nei",
            "maid_status_yotogiclass_enabled_list",
        )
    };
    let table = state.table(table)?;
    let id = (1..table.rows)
        .find(|&r| table.string(1, r) == unique_name)
        .map(|r| table.int(0, r))
        .with_context(|| format!("unknown skill class {unique_name}"))?;
    Ok(state.enabled(list, id)?
        || legacy && state.legacy_ids("yotogi_class_enabled_list")?.contains(&id))
}

pub(crate) fn catalogue(s: &mut TjsState, legacy: bool) -> Result<Catalogue> {
    if legacy {
        require_transferred_character(s)?;
    }
    let manager = s
        .retail
        .selection
        .as_ref()
        .context("no active Yotogi manager")?;
    ensure!(manager.legacy == legacy, "Yotogi profile mismatch");
    let previous = manager.free_choice.clone();
    let lead = s
        .active_maids
        .first()
        .copied()
        .flatten()
        .context("free skill selection requires an active character")?;
    let maid = s
        .maids
        .get(&lead)
        .context("free skill lead character status")?;
    let extra = s.retail.characters.get(&maid.guid_id);
    let owned: BTreeSet<_> = if legacy {
        extra
            .map(|status| status.old_skill_ids.clone())
            .unwrap_or_default()
    } else {
        maid.skill_data.iter().map(|(&id, _)| id).collect()
    };
    let experience = usize::try_from(maid.seikeiken)
        .ok()
        .filter(|&v| v < 4)
        .context("invalid free skill experience state")?;
    let relation = extra.map_or(0, |status| status.relation);
    let cast = s
        .active_maids
        .iter()
        .take_while(|slot| slot.is_some())
        .count();
    let stages = stages(s, legacy)?;
    let (details, acquisition) = if legacy {
        (
            Arc::new(s.retail.legacy_table("yotogi_skil_detail.nei")?),
            Arc::new(s.retail.legacy_table("yotogi_skil_acquisition.nei")?),
        )
    } else {
        (
            s.retail.table("yotogi_skill_list.nei")?,
            s.retail.table("yotogi_skill_acquisition.nei")?,
        )
    };
    let old_enabled = if legacy {
        s.retail.legacy_ids("yotogi_skil_list")?
    } else {
        BTreeSet::new()
    };
    let mark = if legacy { "○" } else { "〇" };
    let mut skills = Vec::new();
    for r in 1..details.rows {
        let id = details.int(0, r);
        if !details.exists(0, r)
            || !owned.contains(&id)
            || if legacy {
                !old_enabled.contains(&id)
            } else {
                !s.retail.enabled("yotogi_skill_enabled_list", id)?
            }
        {
            continue;
        }
        let (num, exp, category, name) = if legacy {
            (12, 13, 2, 3)
        } else {
            (13, 15, 3, 4)
        };
        if details.int(num, r).max(1) as usize > cast || details.string(exp + experience, r) != mark
        {
            continue;
        }
        // IsExecContract checks map membership. All three native contract keys
        // are inserted, including the entries with a false value.
        if !legacy
            && (!(0..3).contains(&relation) || details.string(19 + relation as usize, r) != mark)
        {
            continue;
        }
        ensure!(
            acquisition.int(0, r) == id,
            "unaligned free skill acquisition for id {id}"
        );
        let classes: &[(usize, bool)] = if legacy {
            &[(15, false)]
        } else {
            &[(14, true), (16, false)]
        };
        let mut enabled = true;
        for &(column, job) in classes {
            if acquisition.exists(1, r) && acquisition.int(column + 1, r) > 0 {
                enabled &=
                    class_enabled(&mut s.retail, &acquisition.string(column, r), job, legacy)?;
            }
        }
        if !enabled {
            continue;
        }
        let category = CATEGORIES
            .iter()
            .position(|&v| details.string(category, r) == v)
            .filter(|&i| legacy || i < 8)
            .with_context(|| format!("unknown category for free skill {id}"))?;
        let kind = if legacy {
            if details.string(22, r) == mark {
                2
            } else {
                usize::from(details.string(23, r) == mark)
            }
        } else {
            // The UI displays Confess before Faint, unlike the enum.
            match details.int(23, r) {
                n @ 0..=3 => n as usize,
                4 => 5,
                5 => 4,
                n => bail!("unknown free skill condition {n}"),
            }
        };
        let allowed: BTreeSet<_> = (25..details.columns)
            .take_while(|&col| details.exists(col, 0))
            .filter(|&col| details.string(col, r) == mark)
            .map(|col| details.string(col, 0))
            .collect();
        if !legacy && !stages.iter().any(|stage| allowed.contains(&stage.key)) {
            continue;
        }
        let order = if legacy { id } else { details.int(2, r) };
        skills.push((
            order,
            Skill {
                id,
                name: details.string(name, r),
                kind,
                category,
                stages: allowed,
            },
        ));
    }
    skills.sort_by_key(|(order, skill)| (skill.category, *order, skill.id));
    let locked = s.player.flag("__lockNTRPlay") != 0;
    s.maids
        .get_mut(&lead)
        .context("free skill lead character status")?
        .cur_excite = 0;
    s.retail
        .selection
        .as_mut()
        .context("no active Yotogi manager")?
        .free = true;
    Ok(Catalogue {
        skills: skills.into_iter().map(|(_, skill)| skill).collect(),
        stages,
        locked,
        previous,
    })
}

pub(crate) fn apply(s: &mut TjsState, choice: Choice, backdrop: &str) -> Result<()> {
    let manager = s
        .retail
        .selection
        .as_mut()
        .context("no active Yotogi manager")?;
    ensure!(
        manager.free && (1..=3).contains(&choice.level),
        "invalid free skill state"
    );
    manager.selected.fill(None);
    manager.selected[0] = Some((choice.skill, None));
    s.stage_name.clone_from(&choice.stage);
    s.stage_prefab = backdrop.to_owned();
    manager.free_choice = Some(choice);
    Ok(())
}

#[cfg(test)]
pub(crate) fn choice(s: &TjsState) -> Option<&Choice> {
    s.retail.selection.as_ref()?.free_choice.as_ref()
}
