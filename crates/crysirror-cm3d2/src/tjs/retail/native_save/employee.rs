// SPDX-License-Identifier: MPL-2.0
use super::{
    super::{State, TjsState, character, preset, row_id, stock},
    player::{ordered, pairs, set},
};
use crate::{
    params::{Exp, MaidStatus, SkillData, WorkData},
    parts_color::PartsColor,
    save::{
        status_1040::{Body, ClassEntries, Experience, Heroine, Parameters, Skill, Status, Work},
        stock_1040::{Employee, Misc},
    },
};
use anyhow::{Context, Result, ensure};
use crysirror_core::{image_processing::RgbaImage, state::ClientId};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const ACTIVE_EMPLOYEES: usize = 18;

macro_rules! status_fields {
    ($map:ident) => {
        $map! {
            plays: yotogi_play_count,
            night_work_count: others_play_count,
            likability: likability,
            study_rate: study_rate,
            max_hp: hp,
            hp: cur_hp,
            max_mind: mind,
            mind: cur_mind,
            max_reason: reason,
            reason: cur_reason,
            reception: reception,
            care: care,
            lovely: lovely,
            elegance: elegance,
            charm: charm,
            desire: inyoku,
            sensitivity: m_value,
            perversion: hentai,
            service: housi,
            teaching: teach_rate,
            people_count: play_number,
            rank: popular_rank,
            evaluation: evaluation,
        }
    };
}

pub(crate) fn restore_employees(
    state: &mut TjsState,
    employees: &[Employee<'_>],
    client: ClientId,
) -> Result<()> {
    let source = state.retail.source()?.clone();
    let mut guids = BTreeSet::new();
    state.maids.clear();
    state.stock.clear();
    state.active_maids = vec![None; ACTIVE_EMPLOYEES];
    state.bodies.clear();
    state.retail.characters.clear();
    state.retail.native_employee_state.clear();
    state.retail.appearances.clear();
    state.retail.stock_models.clear();
    state.retail.main_characters.clear();
    state.next_character = 0;
    for employee in employees {
        let saved = &employee.status;
        ensure!(
            !saved.guid.is_empty() && guids.insert(saved.guid.clone()),
            "missing or duplicate stock employee identity"
        );
        let id = state.next_character;
        state.next_character += 1;
        let mut model = stock::retail_maid(false);
        let appearance =
            preset::restore_model_props(&mut model, &employee.properties, Some(employee.colors))?;
        model.name_first.clone_from(&saved.first_name);
        model.name_last.clone_from(&saved.last_name);
        model.man_color = employee.misc.color;
        model
            .all_proc_prop(&source.files, source.storage.as_ref())
            .context("restore saved employee model")?;
        state.bodies.insert(id, stock::mirror(&model));

        let (mut params, mut extra) = restore_status(&mut state.retail, saved)?;
        let jobs = class_names(&mut state.retail, &extra.job_levels, "maid_status_jobclass")?;
        stock::class_bonus(&mut state.retail, &mut params, &mut extra, &jobs)?;
        extra
            .thumbnail_card_time
            .clone_from(&employee.misc.thumbnail_time);
        if !employee.misc.thumbnail.is_empty() {
            // ThumShot.m_siIconSize is 90 by 90 in 1.0.4.
            let png = employee.misc.thumbnail.as_ref();
            ensure!(png.len() <= 128 * 1024, "invalid 1.0.4 stock icon length");
            let header = crysirror_image::png_header::PngHeader::read(png, &|| Ok(()))?;
            ensure!(
                header.size == [90, 90],
                "invalid 1.0.4 stock icon dimensions"
            );
            let image = crysirror_image::decode_standard(png)?;
            extra.thumbnail_icon = Some(Arc::new(RgbaImage::new(
                client,
                [image.width, image.height],
                image.rgba.into(),
            )?));
        }
        if employee.misc.active_slot != -1 {
            let slot = usize::try_from(employee.misc.active_slot)?;
            let current = state
                .active_maids
                .get_mut(slot)
                .context("invalid saved active maid slot")?;
            ensure!(
                current.replace(id).is_none(),
                "duplicate saved active maid slot"
            );
        }
        if saved.main_character {
            state.retail.main_characters.insert(id);
        }
        state.retail.native_employee_state.insert(
            saved.guid.clone(),
            (saved.clone(), employee.misc.to_owned()),
        );
        state.retail.characters.insert(saved.guid.clone(), extra);
        state.maids.insert(id, params);
        state.retail.stock_models.insert(id, Arc::new(model));
        state.retail.appearances.insert(id, appearance);
        state.stock.push(id);
    }
    for slot in &state.player.schedule_slot {
        ensure!(
            slot.maid_guid.is_empty() || guids.contains(&slot.maid_guid),
            "schedule references a missing employee"
        );
    }
    Ok(())
}

fn restore_status(retail: &mut State, saved: &Status) -> Result<(MaidStatus, character::Status)> {
    let p = &saved.parameters;
    let b = &saved.body;
    let mut maid = MaidStatus {
        guid_id: saved.guid.clone(),
        create_time: saved.creation_time.clone(),
        create_time_num: saved.creation_time.parse().unwrap_or(0),
        employment_day: saved.employment_day,
        last_name: saved.last_name.clone(),
        first_name: saved.first_name.clone(),
        profile: saved.profile.clone(),
        free_comment: saved.comment.clone(),
        init_seikeiken: saved.initial_experience.into(),
        seikeiken: saved.experience_type.into(),
        personal: saved.personality,
        contract_type: saved.contract.into(),
        current_maid_class: saved.selected_job_class.unwrap_or_default(),
        current_yotogi_class: saved.selected_night_class.unwrap_or_default(),
        feature: set(&saved.features),
        propensity: set(&saved.propensities),
        body: crate::params::Body {
            height: b.height.into(),
            weight: b.weight.into(),
            bust: b.bust.into(),
            waist: b.waist.into(),
            hip: b.hip.into(),
            cup: b.cup.clone(),
        },
        sales: p.sales,
        total_sales: p.total_sales,
        total_evaluation: p.total_evaluation,
        leader: p.leader,
        is_first_name_call: p.first_name_call,
        eye_parts_tab: p.eye_parts_tab.into(),
        noon_work_id: p.day_job,
        night_work_id: p.night_job,
        generic_flag: ordered(&saved.flags),
        parts_dic: ordered(&saved.parts),
        employment: true,
        ..Default::default()
    };
    macro_rules! restore_values {
        ($($disk:ident: $live:ident,)*) => {
            $(maid.$live = p.$disk.into();)*
        };
    }
    status_fields!(restore_values);
    let mut extra = character::Status {
        relation: saved.relation.into(),
        cooking: p.cooking.into(),
        vocal: p.vocal.into(),
        dance: p.dance.into(),
        appeal: p.appeal.into(),
        communication: [p.day_communication, p.night_communication],
        ..Default::default()
    };
    if let Heroine::Sub(id) = saved.heroine {
        let table = retail.table("maid_status_submaid_list.nei")?;
        extra.sub_name = Some(table.string(1, row_id(&table, id)?));
    }
    for (id, job) in &saved.job_classes.current {
        extra.job_levels.insert(*id, job.level);
        extra.class_exp[0].insert(*id, job.total);
    }
    for (id, job) in &saved.night_classes.current {
        extra.skill_class_levels.insert(*id, job.level);
        extra.class_exp[1].insert(*id, job.total);
    }
    let skills = retail.table("yotogi_skill_list.nei")?;
    for (id, skill) in &saved.skills.current {
        let row = row_id(&skills, *id)?;
        let exp = &skill.experience;
        maid.skill_data.insert(
            *id,
            SkillData {
                id: *id,
                play_count: skill.counters.plays,
                exp: Exp {
                    current: exp.current,
                    total: exp.total,
                    next: exp.next,
                    level: exp.level,
                    need: vec![0, skills.int(10, row), skills.int(11, row)],
                },
            },
        );
    }
    extra.old_skill_ids = saved.skills.legacy.iter().map(|(id, _)| *id).collect();
    for (id, work) in &saved.works {
        maid.work_data.insert(
            *id,
            WorkData {
                id: work.id,
                play_count: work.plays,
                level: work.level,
            },
        );
    }
    Ok((maid, extra))
}

pub(crate) fn capture_employees(state: &mut TjsState) -> Result<Vec<Employee<'static>>> {
    let ids = state.stock.clone();
    ids.into_iter()
        .map(|id| {
            let m = state
                .maids
                .get(&id)
                .with_context(|| format!("missing employee status {id}"))?
                .clone();
            let extra = state
                .retail
                .characters
                .get(&m.guid_id)
                .cloned()
                .unwrap_or_default();
            let template = state.retail.native_employee_state.get(&m.guid_id).cloned();
            let status = capture_status(
                &mut state.retail,
                id,
                &m,
                &extra,
                template.as_ref().map(|v| &v.0),
            )?;
            let active_slot = state
                .active_maids
                .iter()
                .position(|entry| *entry == Some(id))
                .map(i32::try_from)
                .transpose()?
                .unwrap_or(-1);
            let model = state
                .retail
                .stock_models
                .get(&id)
                .context("missing saved employee appearance")?;
            let appearance = state.retail.appearances.get(&id);
            let properties = preset::capture_model_props(model, appearance)?;
            let colors = appearance
                .and_then(|appearance| appearance.colors)
                .unwrap_or_else(|| {
                    let mut colors = [PartsColor::default(); 9];
                    colors[..7].copy_from_slice(&model.parts_color);
                    if let Some(outline) = model.outline_colors {
                        colors[7..].copy_from_slice(&outline);
                    }
                    colors
                });
            let mut misc = template.map(|(_, misc)| misc).unwrap_or_else(|| Misc {
                active_slot,
                thumbnail: Cow::Borrowed(&[]),
                thumbnail_time: String::new(),
                color: model.man_color,
                voice_pitch: 50,
            });
            misc.active_slot = active_slot;
            if let Some(icon) = extra.thumbnail_icon.as_ref()
                && (misc.thumbnail.is_empty() || misc.thumbnail_time != extra.thumbnail_card_time)
            {
                let [width, height] = icon.size();
                misc.thumbnail = Cow::Owned(crysirror_image::encode_png(
                    &crysirror_image::ImageSurface {
                        width,
                        height,
                        rgba: icon.rgba().to_vec(),
                    },
                )?);
            }
            misc.thumbnail_time.clone_from(&extra.thumbnail_card_time);
            misc.color = model.man_color;
            Ok(Employee {
                properties,
                colors,
                status,
                misc,
            })
        })
        .collect()
}

fn capture_status(
    retail: &mut State,
    id: usize,
    m: &MaidStatus,
    extra: &character::Status,
    template: Option<&Status>,
) -> Result<Status> {
    let mut saved = template.cloned().unwrap_or_default();
    saved.guid.clone_from(&m.guid_id);
    saved.creation_time.clone_from(&m.create_time);
    if let Some(sub) = &extra.sub_name {
        let t = retail.table("maid_status_submaid_list.nei")?;
        let r = (1..t.rows)
            .find(|&r| t.string(1, r) == *sub)
            .context("unknown saved subcharacter")?;
        saved.heroine = Heroine::Sub(t.int(0, r));
    }
    saved.main_character = retail.main_characters.contains(&id);
    saved.employment_day = m.employment_day;
    saved.last_name.clone_from(&m.last_name);
    saved.first_name.clone_from(&m.first_name);
    saved.profile.clone_from(&m.profile);
    saved.comment.clone_from(&m.free_comment);
    saved.initial_experience = m.init_seikeiken.try_into()?;
    saved.experience_type = m.seikeiken.try_into()?;
    saved.personality = m.personal;
    saved.contract = m.contract_type.try_into()?;
    saved.body = Body {
        height: m.body.height.try_into()?,
        weight: m.body.weight.try_into()?,
        bust: m.body.bust.try_into()?,
        waist: m.body.waist.try_into()?,
        hip: m.body.hip.try_into()?,
        cup: m.body.cup.clone(),
    };
    saved.relation = extra.relation.try_into()?;
    let mut p = Parameters {
        sales: m.sales,
        total_sales: m.total_sales,
        total_evaluation: m.total_evaluation,
        leader: m.leader,
        first_name_call: m.is_first_name_call,
        status_lock: saved.parameters.status_lock,
        eye_parts_tab: m.eye_parts_tab.try_into()?,
        day_job: m.noon_work_id,
        night_job: m.night_work_id,
        day_communication: extra.communication[0],
        night_communication: extra.communication[1],
        cooking: extra.cooking.try_into()?,
        vocal: extra.vocal.try_into()?,
        dance: extra.dance.try_into()?,
        appeal: extra.appeal.try_into()?,
        ..Default::default()
    };
    macro_rules! capture_values {
        ($($disk:ident: $live:ident,)*) => {
            $(p.$disk = m.$live.try_into().with_context(|| format!("native employee parameter {}", stringify!($disk)))?;)*
        };
    }
    status_fields!(capture_values);
    saved.parameters = p;
    saved.features = m.feature.iter().map(|(id, ())| *id).collect();
    saved.propensities = m.propensity.iter().map(|(id, ())| *id).collect();
    saved.flags = pairs(&m.generic_flag);
    saved.parts = pairs(&m.parts_dic);
    saved.works = m
        .work_data
        .iter()
        .map(|(&id, work)| {
            (
                id,
                Work {
                    id: work.id,
                    plays: work.play_count,
                    level: work.level,
                },
            )
        })
        .collect();
    sync_classes(
        retail,
        &mut saved.job_classes,
        &extra.job_levels,
        &extra.class_exp[0],
        "maid_status_jobclass",
    )?;
    sync_classes(
        retail,
        &mut saved.night_classes,
        &extra.skill_class_levels,
        &extra.class_exp[1],
        "maid_status_yotogiclass",
    )?;
    saved.selected_job_class = extra
        .job_levels
        .contains_key(&m.current_maid_class)
        .then_some(m.current_maid_class);
    saved.selected_night_class = extra
        .skill_class_levels
        .contains_key(&m.current_yotogi_class)
        .then_some(m.current_yotogi_class);

    let previous = std::mem::take(&mut saved.skills.current)
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    saved.skills.current = m
        .skill_data
        .iter()
        .map(|(&id, skill)| {
            let mut capture = previous.get(&id).cloned().unwrap_or_else(Skill::default);
            capture.counters.plays = skill.play_count;
            capture.experience = experience(&skill.exp);
            (id, capture)
        })
        .collect();
    saved.skills.current.sort_by_key(|v| v.0);
    saved.skills.legacy.retain(|(id, record)| {
        record.is_some()
            || saved
                .skills
                .current
                .iter()
                .any(|(current, _)| current == id)
    });
    Ok(saved)
}

fn experience(exp: &Exp) -> Experience {
    Experience {
        current: exp.current,
        total: exp.total,
        next: exp.next,
        level: exp.level,
    }
}

fn class_names(
    retail: &mut State,
    levels: &BTreeMap<i32, i32>,
    family: &str,
) -> Result<BTreeMap<String, i32>> {
    let t = retail.table(&format!("{family}_list.nei"))?;
    levels
        .iter()
        .map(|(&id, &level)| Ok((t.string(1, row_id(&t, id)?), level)))
        .collect()
}

fn sync_classes(
    retail: &mut State,
    entries: &mut ClassEntries<Experience>,
    levels: &BTreeMap<i32, i32>,
    totals: &BTreeMap<i32, i32>,
    family: &str,
) -> Result<()> {
    let previous = std::mem::take(&mut entries.current)
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let names = retail.table(&format!("{family}_list.nei"))?;
    let exp = retail.table(&format!("{family}_experiences.nei"))?;
    for (&id, &level) in levels {
        let total = *totals.get(&id).context("missing class total experience")?;
        if let Some(saved) = previous
            .get(&id)
            .filter(|saved| saved.total == total && saved.level == level)
        {
            entries.current.push((id, saved.clone()));
            continue;
        }
        let key = names.string(1, row_id(&names, id)?);
        let row = (1..exp.rows)
            .find(|&row| exp.string(0, row) == key)
            .context("missing class experience table")?;
        let mut xp = Exp::default();
        xp.set_list(
            (1..exp.columns)
                .take_while(|&col| exp.exists(col, row))
                .map(|col| exp.int(col, row))
                .collect(),
        );
        xp.add(total);
        ensure!(xp.level == level, "inconsistent native class level {id}");
        entries.current.push((id, experience(&xp)));
    }
    entries
        .legacy
        .retain(|(id, record)| record.is_some() || levels.contains_key(id));
    Ok(())
}
