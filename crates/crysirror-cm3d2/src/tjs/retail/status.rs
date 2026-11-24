// SPDX-License-Identifier: MPL-2.0
use super::*;
use character::Status;

fn enum_text(names: &[&str], value: i32) -> Variant {
    Variant::String(names.get(value as usize).copied().unwrap_or("").into())
}
pub(super) fn read(s: &mut TjsState, a: &[Variant]) -> Result<Variant> {
    arity(a, 2)?;
    let slot = arg_int(a, 0) as usize;
    let legacy = s.retail.compatibility || s.player.flag("2.0モード") == 1;
    let name = arg_string(a, 1);
    let name = if legacy && name == "状態" {
        "旧関係"
    } else {
        &name
    };
    let i = s
        .active_maids
        .get(slot)
        .copied()
        .flatten()
        .context("empty maid slot")?;
    let m = s.maids.get(&i).context("missing maid status")?;
    let default = Status::default();
    let x = s.retail.characters.get(&m.guid_id).unwrap_or(&default);
    let relations = ["お近づき", "信頼", "恋人"];
    let experiences = ["処女", "経験あり", "後ろ経験あり", "両方経験あり"];
    Ok(match name {
        "性経験" => enum_text(&experiences, m.seikeiken),
        "初期性経験" => enum_text(&experiences, m.init_seikeiken),
        "性格" => {
            let t = s.retail.table("maid_status_personal_list.nei")?;
            Variant::String(t.string(2, row_id(&t, m.personal)?))
        }
        "契約タイプ" => enum_text(&["育成中", "フリー", "専属"], m.contract_type),
        "ヒロインタイプ" => Variant::String(
            if x.sub_name.is_some() {
                "サブ"
            } else if x.old.is_some() {
                "移籍"
            } else {
                "ユーザー作成"
            }
            .into(),
        ),
        "サブメイドユニークネーム" => {
            Variant::String(x.sub_name.clone().unwrap_or_default())
        }
        "関係" => enum_text(&relations, x.relation),
        "旧関係" => x.old.as_ref().map_or_else(
            || enum_text(&relations, x.relation),
            |old| {
                enum_text(
                    &["", "緊張", "お近づき", "信頼", "恋人", "愛奴"],
                    old.relation,
                )
            },
        ),
        "ジョブメイド" | "夜伽クラス" | "ジョブメイドレベル" | "夜伽クラスレベル" =>
        {
            let skill = name == "夜伽クラスレベル";
            let id = if skill {
                m.current_yotogi_class
            } else {
                m.current_maid_class
            };
            let level = if skill {
                &x.skill_class_levels
            } else {
                &x.job_levels
            }
            .get(&id)
            .context("missing selected class")?;
            if name.ends_with("レベル") {
                int(*level)
            } else if name == "夜伽クラス" {
                // Every job definition in the unlinked standalone class catalog has ClassType.New.
                Variant::String("Debut".into())
            } else {
                let t = s.retail.table("maid_status_jobclass_list.nei")?;
                Variant::String(t.string(1, row_id(&t, id)?))
            }
        }
        "カップ" => Variant::String(m.body.cup.clone()),
        "苗字" => Variant::String(m.last_name.clone()),
        "名前" => Variant::String(m.first_name.clone()),
        "身長" => int(m.body.height),
        "夜伽プレイ回数" => int(m.yotogi_play_count),
        "接客プレイ回数" => int(m.others_play_count),
        "興奮値" => int(m.cur_excite),
        "習得率" => int(m.study_rate),
        "好感度" => int(if legacy {
            m.likability.min(299)
        } else {
            m.likability
        }),
        "接待" | "お世話" | "可憐" | "淫欲" | "気品" | "M性" | "魅惑" | "変態" | "奉仕"
        | "料理" | "ボーカル" | "ダンス" | "指導" | "経験人数" => {
            int(character::parameter(m, x, name)?)
        }
        "アピール" => int(character::parameter(m, x, "アピールポイント")?),
        "昼仕事ID" => int(m.noon_work_id),
        "夜仕事ID" => int(m.night_work_id),
        "昼仕事レベル" | "夜仕事レベル" => int(m
            .work_data
            .get(&if name.starts_with('昼') {
                m.noon_work_id
            } else {
                m.night_work_id
            })
            .map_or(1, |w| w.level)),
        "レンタルメイド" if legacy => int(0),
        "雇用中" if legacy => int(1),
        "特殊状態" if legacy => enum_text(
            &["なし", "酔い", "お仕置き"],
            x.old
                .as_ref()
                .context("missing transferred status")?
                .condition,
        ),
        "欲求不満" if legacy => {
            let old = x.old.as_ref().context("missing transferred status")?;
            int(old.frustration)
        }
        "淫乱ギャップ値" if legacy => int(m.a_inyoku() + m.a_lovely()),
        "M性ギャップ値" if legacy => int(m.a_m_value() + m.a_elegance()),
        "変態ギャップ値" if legacy => int(m.a_hentai() + m.a_charm()),
        "メイドクラス" if legacy => {
            ensure!(
                x.job_levels.contains_key(&m.current_maid_class),
                "missing selected class"
            );
            Variant::String("ノービスメイド".into())
        }
        _ => Variant::Void,
    })
}

pub(super) fn flag(s: &mut TjsState, a: &[Variant], write: bool) -> Result<Variant> {
    let n = if write { 2 } else { 1 };
    ensure!(
        a.len() == n || a.len() == n + 1,
        "expected optional slot, flag name and value for a write"
    );
    let offset = usize::from(a.len() != n);
    let slot = if offset != 0 {
        arg_int(a, 0) as usize
    } else {
        0
    };
    let key = arg_string(a, offset);
    let legacy = s.retail.compatibility || s.player.flag("2.0モード") == 1;
    let index = s
        .active_maids
        .get(slot)
        .copied()
        .flatten()
        .context("empty maid slot")?;
    let m = s.maids.get_mut(&index).context("no maid status")?;
    let flags = if legacy {
        &mut s
            .retail
            .characters
            .get_mut(&m.guid_id)
            .and_then(|c| c.old.as_mut())
            .context("missing transferred status")?
            .flags
    } else {
        &mut m.generic_flag
    };
    if write {
        let v = arg_int(a, offset + 1) as i32;
        flags.insert(key, v);
        Ok(int(v))
    } else {
        Ok(int(flags.get(&key).copied().unwrap_or(0)))
    }
}
