// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::params::MaidStatus;

#[derive(Clone, Default)]
pub(super) struct Status {
    pub(super) portraits: Option<[String; 2]>,
    pub(super) relation: i32,
    pub(super) old_skill_ids: BTreeSet<i32>,
    pub(super) sub_name: Option<String>,
    pub(super) sensual: i32,
    pub(super) communication: [bool; 2],
    pub(super) job_levels: BTreeMap<i32, i32>,
    pub(super) skill_class_levels: BTreeMap<i32, i32>,
    pub(super) old: Option<LegacyStatus>,
    pub(super) cooking: i32,
    pub(super) vocal: i32,
    pub(super) dance: i32,
    pub(super) appeal: i32,
    pub(super) bonus_excite: i32,
    pub(super) bonus_sensual: i32,
    pub(super) bonus_reason: i32,
    pub(super) bonus_cooking: i32,
    pub(super) bonus_vocal: i32,
    pub(super) bonus_dance: i32,
    pub(super) bonus_appeal: i32,
}

pub(super) fn parameter(m: &MaidStatus, extra: &Status, name: &str) -> Result<i32> {
    Ok(match name {
        "接待" => m.a_reception(),
        "お世話" => m.a_care(),
        "可憐" => m.a_lovely(),
        "気品" => m.a_elegance(),
        "魅惑" => m.a_charm(),
        "淫欲" => m.a_inyoku(),
        "M性" => m.a_m_value(),
        "変態" => m.a_hentai(),
        "奉仕" => m.a_housi(),
        "指導" => (m.teach_rate + m.bonus.teach_rate).clamp(0, 9999),
        "経験人数" => m.play_number,
        "人気ランク" => m.popular_rank,
        "評価" => m.evaluation as i32,
        // The retail cooking getter uses bonusStatus.charm.
        "料理" => (extra.cooking + m.bonus.charm).clamp(0, 9999),
        "ボーカル" => (extra.vocal + extra.bonus_vocal).clamp(0, 9999),
        "ダンス" => (extra.dance + extra.bonus_dance).clamp(0, 9999),
        "アピールポイント" => {
            let dance = (extra.dance + extra.bonus_dance).clamp(0, 9999);
            (extra.appeal + extra.bonus_appeal).clamp(0, 200 + dance / 1000 * 200)
        }
        _ => bail!("unknown play condition parameter {name}"),
    })
}

pub(super) fn eligible(s: &mut TjsState, a: &[Variant]) -> Result<Variant> {
    arity(a, 2)?;
    let slot = arg_int(a, 0) as usize;
    ensure!(s.maid(slot).is_some(), "empty maid slot {slot}");
    let t = s.retail.table("kasizuki_play_list.nei")?;
    let r = row_id(&t, arg_int(a, 1) as i32)?;
    ensure!(
        matches!(number(&s.retail, "男種類"), 10 | 20),
        "unknown guest type"
    );
    if likability(s, slot)? < t.int(16, r) {
        return Ok(boolean(false));
    }
    let names = s.retail.table("kasizuki_work_count_convert_list.nei")?;
    let tiers: BTreeMap<_, _> = (1..names.rows)
        .map(|i| (names.int(0, i), names.string(1, i)))
        .collect();
    let marked = |col| matches!(t.string(col, r).as_str(), "〇" | "○");
    let allowed: Vec<_> = tiers
        .values()
        .enumerate()
        .filter_map(|(i, label)| marked(17 + i).then_some(label))
        .collect();
    if !allowed.is_empty() {
        let label = threshold_name(&names, maid_flag(s, slot, "仕事回数")?)?;
        if !allowed.contains(&&label) {
            return Ok(boolean(false));
        }
    }
    let m = s.maid(slot).context("missing maid")?;
    let defaults = Status::default();
    let extra = s.retail.characters.get(&m.guid_id).unwrap_or(&defaults);
    let conditions = t.string(10, r);
    if !conditions.is_empty() {
        for cond in conditions.split(',') {
            let (name, min) = cond.split_once('|').context("malformed play condition")?;
            if parameter(m, extra, name)? < min.trim().parse::<i32>()? {
                return Ok(boolean(false));
            }
        }
    }
    let skills = t.string(15, r);
    if !skills.is_empty() {
        for id in skills.split(',') {
            if m.skill_data.get(&id.trim().parse::<i32>()?).is_none() {
                return Ok(boolean(false));
            }
        }
    }
    if !(0..4).contains(&m.seikeiken) || !marked(11 + m.seikeiken as usize) {
        return Ok(boolean(false));
    }
    let rel = 17 + tiers.len();
    Ok(boolean(
        !(0..3).any(|i| marked(rel + i))
            || ((0..3).contains(&extra.relation) && marked(rel + extra.relation as usize)),
    ))
}

pub(super) fn add_status(s: &mut TjsState, a: &[Variant]) -> Result<Variant> {
    arity(a, 3)?;
    let m = maid(s, a)?;
    let (field, max) = match arg_string(a, 1).as_str() {
        "夜伽プレイ回数" | "接客プレイ回数" => (&mut m.others_play_count, 9999),
        "好感度" => (&mut m.likability, 999),
        "習得率" => (&mut m.study_rate, 1000),
        "接待" => (&mut m.reception, 9999),
        "お世話" => (&mut m.care, 9999),
        "可憐" => (&mut m.lovely, 9999),
        "気品" => (&mut m.elegance, 9999),
        "魅惑" => (&mut m.charm, 9999),
        "淫欲" => (&mut m.inyoku, 9999),
        "M性" => (&mut m.m_value, 9999),
        "変態" => (&mut m.hentai, 9999),
        "奉仕" => (&mut m.housi, 9999),
        "指導" => (&mut m.teach_rate, 9999),
        "経験人数" => (&mut m.play_number, 999),
        _ => return Ok(Variant::Void),
    };
    *field = field.wrapping_add(arg_int(a, 2) as i32).clamp(0, max);
    Ok(Variant::Void)
}

#[derive(Clone, Default)]
pub(super) struct LegacyStatus {
    pub(super) sexual: crate::params::Sexual,
    pub(super) relation: i32,
    pub(super) condition: i32,
    pub(super) frustration: i32,
    pub(super) flags: crate::binio::OrderedMap<String, i32>,
    marriage: bool,
    new_wife: bool,
}

pub(super) fn legacy_sensitivity(s: &mut TjsState, a: &[Variant]) -> Result<Variant> {
    ensure!(s.retail.compatibility || s.player.flag("2.0モード") == 1, "requires legacy mode");
    arity(a, 2)?;
    let maid = s.maid(arg_int(a, 0) as usize).context("invalid maid slot")?;
    let legacy = s.retail.characters.get(&maid.guid_id).and_then(|c| c.old.as_ref()).context("maid has no transferred status")?;
    let values = &legacy.sexual;
    let value = match arg_string(a, 1).as_str() {
        "口" => values.mouth, "喉" => values.throat, "乳首" => values.nipple,
        "前穴" => values.front, "後穴" => values.back, "クリ" => values.curi,
        other => bail!("unknown IsSeikantai tag {other}"),
    };
    Ok(boolean(value >= 1000))
}

pub(super) fn set_experience(s: &mut TjsState, a: &[Variant], bit: i32) -> Result<Variant> {
    arity(a, 2)?;
    let value = arg_bool(a, 1);
    let m = maid(s, a)?;
    // OnChangeSeikeiken is empty in the retail manager. The old manager
    // refreshes only its VR selection menu; this standalone host runs in 2D.
    m.seikeiken = if value { m.seikeiken | bit } else { m.seikeiken & !bit };
    Ok(Variant::Void)
}

pub(super) fn old_flag(s: &mut TjsState, a: &[Variant], marriage: bool) -> Result<Variant> {
    ensure!(
        s.retail.compatibility || s.player.flag("2.0モード") == 1,
        "requires legacy mode"
    );
    arity(a, 1)?;
    let old = s
        .maid(arg_int(a, 0) as usize)
        .and_then(|m| s.retail.characters.get(&m.guid_id))
        .and_then(|m| m.old.as_ref());
    Ok(boolean(old.is_some_and(|m| {
        if marriage { m.marriage } else { m.new_wife }
    })))
}

pub(super) fn set_condition(s: &mut TjsState, a: &[Variant]) -> Result<Variant> {
    arity(a, 2)?;
    let legacy = s.retail.compatibility || s.player.flag("2.0モード") == 1;
    let guid = maid(s, a)?.guid_id.clone();
    let status = s.retail.characters.entry(guid).or_default();
    let name = arg_string(a, 1);
    if legacy {
        let relation = ["", "緊張", "お近づき", "信頼", "恋人", "愛奴"]
            .iter()
            .skip(1)
            .position(|n| *n == name);
        if let Some(r) = relation {
            status
                .old
                .as_mut()
                .context("maid has no transferred status")?
                .relation = r as i32 + 1;
        }
        let condition = match name.as_str() {
            "酔い" => Some(1),
            "お仕置き" => Some(2),
            "酔い解除" | "お仕置き解除" => Some(0),
            _ => None,
        };
        if let Some(c) = condition {
            status
                .old
                .as_mut()
                .context("maid has no transferred status")?
                .condition = c;
        }
    } else if let Some(r) = ["お近づき", "信頼", "恋人"].iter().position(|n| *n == name) {
        status.relation = r as i32;
        if let Some(old) = &mut status.old {
            old.relation = r as i32 + 2;
        }
    }
    Ok(Variant::Void)
}

pub(crate) const MPN: &[&str] = &[
    "null_mpn",
    "MuneL",
    "MuneS",
    "MuneTare",
    "RegFat",
    "ArmL",
    "Hara",
    "RegMeet",
    "KubiScl",
    "UdeScl",
    "EyeScl",
    "EyeSclX",
    "EyeSclY",
    "EyePosX",
    "EyePosY",
    "EyeClose",
    "EyeBallPosX",
    "EyeBallPosY",
    "EyeBallSclX",
    "EyeBallSclY",
    "FaceShape",
    "MayuX",
    "MayuY",
    "HeadX",
    "HeadY",
    "DouPer",
    "sintyou",
    "koshi",
    "kata",
    "west",
    "MuneUpDown",
    "MuneYori",
    "MuneYawaraka",
    "body",
    "moza",
    "head",
    "hairf",
    "hairr",
    "hairt",
    "hairs",
    "hairaho",
    "haircolor",
    "skin",
    "acctatoo",
    "accnail",
    "underhair",
    "hokuro",
    "mayu",
    "lip",
    "eye",
    "eye_hi",
    "chikubi",
    "chikubicolor",
    "wear",
    "skirt",
    "mizugi",
    "bra",
    "panz",
    "stkg",
    "shoes",
    "headset",
    "glove",
    "acchead",
    "accha",
    "acchana",
    "acckamisub",
    "acckami",
    "accmimi",
    "accnip",
    "acckubi",
    "acckubiwa",
    "accheso",
    "accude",
    "accashi",
    "accsenaka",
    "accshippo",
    "accanl",
    "accvag",
    "megane",
    "accxxx",
    "handitem",
    "acchat",
    "onepiece",
    "set_maidwear",
    "set_mywear",
    "set_underwear",
    "set_body",
    "folder_eye",
    "folder_mayu",
    "folder_underhair",
    "folder_skin",
    "kousoku_upper",
    "kousoku_lower",
    "seieki_naka",
    "seieki_hara",
    "seieki_face",
    "seieki_mune",
    "seieki_hip",
    "seieki_ude",
    "seieki_ashi",
];

pub(super) fn mpn_name(input: &str) -> Result<&'static str> {
    enum_value(MPN, input)
        .and_then(|i| MPN.get(i as usize).copied())
        .with_context(|| format!("unknown retail MPN {input}"))
}

pub(super) fn prop(s: &mut TjsState, a: &[Variant], file: bool) -> Result<Variant> {
    let [Variant::Integer(slot), Variant::String(name)] = a else {
        bail!("expected integer slot and MPN name");
    };
    let name = mpn_name(name)?;
    let index = s
        .active_maids
        .get(*slot as usize)
        .copied()
        .flatten()
        .context("empty maid slot")?;
    let body = s.bodies.get(&index).context("missing maid body")?;
    let &(kind, value) = body
        .props
        .get(name)
        .with_context(|| format!("uninitialized MPN {name}"))?;
    Ok(match (kind, file) {
        (3, true) => Variant::String(body.menu_files.get(name).cloned().unwrap_or_default()),
        (3, false) => Variant::String(name.into()),
        (_, true) => Variant::String(String::new()),
        (_, false) => int(value),
    })
}

pub(super) fn class(
    s: &mut TjsState,
    a: &[Variant],
    job: bool,
    level: bool,
    legacy_call: bool,
) -> Result<Variant> {
    if legacy_call {
        ensure!(
            s.retail.compatibility || s.player.flag("2.0モード") == 1,
            "requires legacy mode"
        );
    }
    let [Variant::Integer(slot), Variant::String(name)] = a else {
        bail!("expected integer slot and class name");
    };
    let m = s.maid(*slot as usize).context("empty maid slot")?;
    let guid = m.guid_id.clone();
    let t = s.retail.table(if job {
        "maid_status_jobclass_list.nei"
    } else {
        "maid_status_yotogiclass_list.nei"
    })?;
    let row = (1..t.rows)
        .find(|&r| t.exists(0, r) && t.string(1, r) == *name)
        .with_context(|| format!("unknown class {name}"))?;
    let id = t.int(0, row);
    let entry = s.retail.characters.get(&guid).and_then(|c| {
        if job {
            c.job_levels.get(&id)
        } else {
            c.skill_class_levels.get(&id)
        }
    });
    Ok(if level {
        int(entry.copied().unwrap_or(0))
    } else {
        boolean(entry.is_some())
    })
}
