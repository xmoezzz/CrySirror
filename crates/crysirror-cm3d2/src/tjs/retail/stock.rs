// SPDX-License-Identifier: MPL-2.0
//! ScriptManager.CreateNewMaid and the standalone CharacterMgr stock.
use super::*;
use crate::{
    chara::{Maid, Prop},
    params::{Exp, MaidStatus, SkillData},
    tjs::{BodyMirror, Request},
    unity_scene::UnityData,
};

pub(crate) fn retail_maid(man: bool) -> Maid {
    let mut maid = Maid::new(man);
    maid.prop_names = character::MPN;
    maid.outline_colors = Some([maid.parts_color[2], maid.parts_color[5]]);
    maid.props = maid
        .prop_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            if i == 0 {
                return Prop {
                    max: i32::MAX,
                    ..Default::default()
                };
            }
            if i > 32 {
                return Prop {
                    kind: 3,
                    dut: true,
                    file: preset::delete_menu(name).unwrap_or("").into(),
                    ..Default::default()
                };
            }
            let (min, max) = match i {
                1 | 3 => (0, 130),
                8 | 25 | 26 => (20, 80),
                _ => (0, 100),
            };
            let value = match i {
                1 | 3 | 30 => 10,
                2 | 15 | 20 => 0,
                4 | 7 | 31 => 40,
                5 | 6 => 20,
                _ => 50,
            };
            Prop {
                min,
                max,
                value,
                default: value,
                kind: if i <= 7 { 1 } else { 2 },
                ..Default::default()
            }
        })
        .collect();
    maid
}

pub(crate) fn prop<'a>(m: &'a Maid, n: &str) -> Result<&'a crate::chara::Prop> {
    Ok(&m.props[m
        .prop_names
        .iter()
        .position(|&name| name == n)
        .context("missing slider property")?])
}
fn slide(m: &mut Maid, n: &str, fraction: f32) -> Result<()> {
    ensure!(fraction.is_finite(), "invalid slider position");
    let fraction = fraction.clamp(0., 1.);
    let p = prop(m, n)?;
    ensure!(
        p.kind == 1 || p.kind == 2,
        "non-numeric editor property {n}"
    );
    let (min, max) = (p.min, p.max);
    if n == "MuneL" {
        let value = ((min - 30) as f32 + (max - min + 30) as f32 * fraction).round() as i32;
        m.set_value(n, value.max(0))?;
        m.set_value("MuneS", (-100. * value.min(0) as f32 / 30.).round() as i32)?;
        let i = m
            .prop_names
            .iter()
            .position(|&x| x == "MuneTare")
            .context("missing MuneTare")?;
        m.props[i].max = value.max(1);
        let tare = if value < 1 { 0 } else { m.props[i].value };
        m.set_value("MuneTare", tare)?;
    } else {
        m.set_value(n, (min as f32 + (max - min) as f32 * fraction + 0.5) as i32)?;
    }
    Ok(())
}

fn named(t: &Table, column: usize, name: &str) -> Result<usize> {
    (1..t.rows)
        .find(|&r| t.string(column, r) == name)
        .with_context(|| format!("unknown stock definition {name}"))
}

fn names(s: &mut State) -> Result<Arc<[Vec<String>; 2]>> {
    if let Some(values) = &s.stock_names {
        return Ok(values.clone());
    }
    let source = s.source()?;
    let path = VfsPath::new(
        Mount::Asset,
        &format!("{}/Managed/Assembly-CSharp.dll", source.dir),
    )?;
    let assembly = crysirror_dotnet::Assembly::parse(source.storage.read(&path)?)?;
    let st = assembly.eval_cctor("MaidRandomName")?;
    let read = |key: &str| -> Result<Vec<String>> {
        let values = st
            .items(st.fields.get(key).context("missing native name array")?)
            .context("invalid native name array")?;
        ensure!(!values.is_empty(), "empty {key}");
        values
            .iter()
            .map(|v| Ok(v.str().context("invalid native character name")?.to_owned()))
            .collect()
    };
    let result = Arc::new([read("first_name_array")?, read("last_name_array")?]);
    s.stock_names = Some(result.clone());
    Ok(result)
}

fn class_parameter(m: &MaidStatus, x: &character::Status, key: &str) -> Result<i32> {
    Ok(match key {
        "興奮" => m.cur_excite,
        "官能" => x.sensual,
        "理性" => (m.reason + x.bonus_reason).clamp(0, 999),
        "体力" => m.a_hp().max(0),
        "精神" => m.a_mind().max(0),
        "AP" => character::parameter(m, x, "アピールポイント")?,
        name => character::parameter(m, x, name)?.max(0),
    })
}

fn apply_class_level(m: &mut MaidStatus, x: &mut character::Status, text: &str) -> Result<()> {
    let mut pack = BTreeMap::<String, i32>::new();
    for term in text.split(',').filter(|v| !v.is_empty()) {
        let term = term
            .replace([' ', '　'], "")
            .replace('＋', "+")
            .replace('ー', "-")
            .replace('＝', "=");
        let operator = ['=', '-', '+']
            .into_iter()
            .find(|&c| term.contains(c))
            .context("invalid class bonus")?;
        let (key, value) = term.split_once(operator).context("missing bonus operand")?;
        let value = value.parse::<i32>()?;
        let field = pack.entry(key.to_owned()).or_default();
        match operator {
            '=' => *field = value,
            '-' => *field -= value,
            _ => *field += value,
        }
    }
    for (key, value) in pack {
        let field = match key.as_str() {
            "興奮" => &mut x.bonus_excite,
            "官能" => &mut x.bonus_sensual,
            "理性" => &mut x.bonus_reason,
            "体力" => &mut m.bonus.hp,
            "精神" => &mut m.bonus.mind,
            "接待" => &mut m.bonus.reception,
            "お世話" => &mut m.bonus.care,
            "可憐" => &mut m.bonus.lovely,
            "気品" => &mut m.bonus.elegance,
            "料理" => &mut x.bonus_cooking,
            "ボーカル" => &mut x.bonus_vocal,
            "ダンス" => &mut x.bonus_dance,
            "AP" => &mut x.bonus_appeal,
            "魅惑" => &mut m.bonus.charm,
            "淫欲" => &mut m.bonus.inyoku,
            "M性" => &mut m.bonus.m_value,
            "変態" => &mut m.bonus.hentai,
            "奉仕" => &mut m.bonus.housi,
            "指導" => &mut m.bonus.teach_rate,
            _ => bail!("unsupported native class bonus {key}"),
        };
        *field += value;
    }
    Ok(())
}

fn initial_classes(
    s: &mut TjsState,
    m: &mut MaidStatus,
    status: &mut character::Status,
    skills: bool,
) -> Result<BTreeMap<String, i32>> {
    let base = if skills {
        "maid_status_yotogiclass"
    } else {
        "maid_status_jobclass"
    };
    let list = s.retail.table(&format!("{base}_list.nei"))?;
    let conditions = s.retail.table(&format!("{base}_acquired_condition.nei"))?;
    let bonus = s.retail.table(&format!("{base}_bonus.nei"))?;
    let facilities = s.retail.table("facility.nei")?;
    let mut levels = BTreeMap::new();
    // GetLearnPossibleClassDatas evaluates the entire list before Add updates the bonuses.
    let mut acquired = Vec::new();
    for row in 1..list.rows {
        if !list.exists(0, row) {
            continue;
        }
        let id = list.int(0, row);
        if !s.retail.enabled(&format!("{base}_enabled_list"), id)? {
            continue;
        }
        let key = list.string(1, row);
        let r = named(&conditions, 0, &key)?;
        let accepts = |col, sep, expect| {
            let v = conditions.string(col, r);
            v.is_empty() || v.split(sep).any(|x: &str| x.trim() == expect)
        };
        if !accepts(1, ',', "Muku")
            || (conditions.string(1, r).is_empty() && conditions.string(2, r) == "〇")
            || !accepts(3, '|', "No_No")
            || !accepts(4, '|', "Trainee")
        {
            continue;
        }
        if conditions.exists(5, r) {
            let mut found = false;
            for id in conditions.string(5, r).split('|') {
                found |= facility::built(s, &facilities, id.trim().parse()?);
            }
            if !found {
                continue;
            }
        }
        let mut eligible = true;
        for c in (6..conditions.columns)
            .step_by(2)
            .take_while(|&c| conditions.exists(c, r))
        {
            let threshold = conditions.int(c + 1, r);
            let ok = if conditions.string(c, r) == "VIP" {
                s.player
                    .night_works_state
                    .get(&threshold)
                    .is_some_and(|w| w.finish)
            } else {
                class_parameter(m, status, &conditions.string(c, r))? >= threshold
            };
            if !ok {
                eligible = false;
                break;
            }
        }
        if eligible {
            acquired.push((id, key));
        }
    }
    for (id, key) in acquired {
        let exp = s.retail.table(&format!("{base}_experiences.nei"))?;
        let er = named(&exp, 0, &key)?;
        let mut xp = Exp::default();
        xp.set_list(
            (1..exp.columns)
                .take_while(|&c| exp.exists(c, er))
                .map(|c| exp.int(c, er))
                .collect(),
        );
        if !skills {
            let br = named(&bonus, 0, &key)?;
            for c in 1..=xp.level as usize {
                apply_class_level(m, status, &bonus.string(c, br))?;
            }
        }
        let (owned, selected, open) = if skills {
            (
                &mut status.skill_class_levels,
                &mut m.current_yotogi_class,
                &mut s.player.yotogi_class_open,
            )
        } else {
            (
                &mut status.job_levels,
                &mut m.current_maid_class,
                &mut s.player.maid_class_open,
            )
        };
        if owned.is_empty() {
            *selected = id;
        }
        owned.insert(id, xp.level);
        open.add(id);
        levels.insert(key, xp.level);
    }
    Ok(levels)
}

fn initial_skills(
    s: &mut State,
    m: &mut MaidStatus,
    jobs: &BTreeMap<String, i32>,
    classes: &BTreeMap<String, i32>,
) -> Result<()> {
    let list = s.table("yotogi_skill_list.nei")?;
    let req = s.table("yotogi_skill_acquisition.nei")?;
    for r in 1..list.rows {
        let id = list.int(0, r);
        if !list.exists(0, r) || !s.enabled("yotogi_skill_enabled_list", id)? {
            continue;
        }
        ensure!(
            id == req.int(0, r),
            "unaligned acquisition row for skill {id}"
        );
        let current = [
            m.a_inyoku(),
            m.a_m_value(),
            m.a_hentai(),
            m.a_housi(),
            m.a_lovely(),
            m.a_elegance(),
            m.a_charm(),
        ];
        if current
            .into_iter()
            .enumerate()
            .any(|(i, v)| v.max(0) < req.int(6 + i, r))
        {
            continue;
        }
        let personals = req.string(13, r);
        if !personals.is_empty() && !personals.split(',').any(|n| n.trim() == "Muku") {
            continue;
        }
        let accepts = if req.int(15, r) > 0 {
            jobs.get(&req.string(14, r))
                .is_some_and(|&v| v >= req.int(15, r))
        } else if req.int(17, r) > 0 {
            classes
                .get(&req.string(16, r))
                .is_some_and(|&v| v >= req.int(17, r))
        } else {
            true
        };
        if accepts {
            let mut exp = Exp::default();
            exp.set_list(vec![0, list.int(10, r), list.int(11, r)]);
            m.skill_data.insert(
                id,
                SkillData {
                    id,
                    exp,
                    ..Default::default()
                },
            );
        }
    }
    Ok(())
}

fn measure(m: &mut MaidStatus, model: &Maid) {
    crate::maid_param::measure_body(
        &mut m.body,
        &|name| model.value(name) as f32,
        5. * (model.value("MuneS") as f32 / 100.),
    );
}
fn mirror(model: &Maid) -> BodyMirror {
    let mut body = BodyMirror::default();
    for (&name, prop) in model.prop_names.iter().zip(&model.props) {
        body.props.insert(name.into(), (prop.kind, prop.value));
        if prop.kind == 3 {
            body.menu_files.insert(name.into(), prop.file.clone());
        }
    }
    body.loaded_slots = model
        .slots
        .iter()
        .filter(|(_, s)| s.model.is_some())
        .map(|(&k, _)| k.into())
        .collect();
    body.params = model
        .slots
        .values()
        .map(|slot| slot.params.clone())
        .collect();
    body
}
pub(crate) fn apply_body(s: &mut TjsState, id: usize, model: &mut Maid) -> Result<()> {
    let source = s.retail.source()?.clone();
    let body = s.bodies.get(&id).context("missing stock body")?;
    for &name in character::MPN {
        if let Some(file) = body.menu_files.get(name) {
            if model
                .prop_names
                .iter()
                .position(|&n| n == name)
                .is_some_and(|i| model.props[i].file != *file)
            {
                model.set_prop_file(name, file, false)?;
            }
        } else if let Some(&(kind, v)) = body.props.get(name)
            && kind != 3
            && model.value(name) != v
        {
            model.set_value(name, v)?;
        }
    }
    s.retail.apply_model_colors(id, model);
    model.all_proc_prop(&source.files, source.storage.as_ref())?;
    s.bodies.insert(id, mirror(model));
    Ok(())
}

fn stem(file: &str) -> &str {
    let file = file.rsplit(['/', '\\']).next().unwrap_or(file);
    file.rsplit_once('.').map_or(file, |v| v.0)
}

fn apply_sub(
    s: &mut TjsState,
    id: usize,
    m: &mut MaidStatus,
    x: &mut character::Status,
    model: &mut Maid,
    sub: &Table,
    row: usize,
) -> Result<()> {
    let key = sub.string(1, row);
    let alt = s.retail.table("maid_status_submaid_kiss_side_list.nei")?;
    let (t, r) = if s.player.flag("__lockUserDraftMaid") != 0 {
        (1..alt.rows)
            .find(|&r| alt.exists(0, r) && alt.int(0, r) == sub.int(0, row))
            .map_or((sub, row), |r| (alt.as_ref(), r))
    } else {
        (sub, row)
    };
    let enum_id = |c, names: &[&str]| {
        names
            .iter()
            .position(|n| **n == t.string(c, r))
            .map(|i| i as i32)
            .context("unknown stock status enum")
    };
    x.sub_name = Some(key);
    m.last_name = t.string(4, r);
    m.first_name = t.string(5, r);
    m.contract_type = enum_id(9, &["Trainee", "Free", "Exclusive"])?;
    m.init_seikeiken = enum_id(13, &["No_No", "Yes_No", "No_Yes", "Yes_Yes"])?;
    m.seikeiken = m.init_seikeiken;
    let v = |c| t.int(c, r).clamp(0, 9999);
    m.likability = v(18).clamp(0, 999);
    m.lovely = v(19);
    m.elegance = v(20);
    m.charm = v(21);
    m.care = v(22);
    m.reception = v(23);
    x.cooking = v(24);
    x.dance = v(25);
    x.vocal = v(26);
    m.others_play_count = v(27);
    m.cur_hp = v(28).min(m.a_hp().max(0));
    m.hp = v(28).min(999);
    m.cur_mind = v(29).min(m.a_mind().max(0));
    m.mind = v(29).min(999);
    m.inyoku = v(30);
    m.m_value = v(31);
    m.hentai = v(32);
    m.housi = v(33);
    m.yotogi_play_count = v(34);
    let source = s.retail.source()?.clone();
    let catalog = s.retail.table("character_preset_basedata.nei")?;
    let row = named(&catalog, 0, stem(&t.string(6, r)))?;
    let preset = catalog.string(3, row);
    let preset = stem(&preset);
    let file = format!("{preset}.preset");
    let bytes = if source.files.exists(&file) {
        source.files.read(source.storage.as_ref(), &file)?
    } else {
        let mut op = crate::adapter::Opener {
            storage: source.storage.clone(),
            dir: source.dir.clone(),
        };
        let mut u = UnityData::new()?;
        u.file(&mut op, "globalgamemanagers")?;
        let k = u
            .resource_of(
                &mut op,
                &format!("Preset/{preset}"),
                crysirror_unity::class_id::TEXT_ASSET,
            )?
            .context("missing sub-character preset")?;
        let (_, b) = u.bytes(&mut op, &k)?;
        crysirror_unity::classes::text_asset(&b)?.1
    };
    preset::apply_bytes(s, id, &bytes)?;
    apply_body(s, id, model)?;
    measure(m, model);
    let portraits = [
        format!("{}.tex", stem(&t.string(7, r))),
        format!("{}.tex", stem(&t.string(8, r))),
    ];
    for file in &portraits {
        crysirror_image::kiss_tex::decode_texture(
            &source.files.read(source.storage.as_ref(), file)?,
        )?;
    }
    x.portraits = Some(portraits);
    Ok(())
}

fn initialize(s: &mut TjsState, key: &str) -> Result<()> {
    let source = s.retail.source()?.clone();
    let personal = s.retail.table("maid_status_personal_list.nei")?;
    let initial = personal.int(0, named(&personal, 1, "Muku")?);
    let target = (1..personal.rows).find(|&r| personal.string(1, r) == key);
    let sub = s.retail.table("maid_status_submaid_list.nei")?;
    let sub_row = if let Some(r) = target {
        ensure!(
            s.retail
                .enabled("maid_status_personal_enabled_list", personal.int(0, r))?,
            "disabled personality {key}"
        );
        None
    } else {
        Some(named(&sub, 1, key)?)
    };
    let id = s.next_character;
    ensure!(
        !s.maids.contains_key(&id) && !s.stock.contains(&id),
        "stock character identity {id} is already allocated"
    );
    let names = names(&mut s.retail)?;
    let stamp = Local::now();
    let mut m = MaidStatus {
        guid_id: s.guid(),
        create_time: stamp.format("%Y%m%d %H:%M:%S").to_string(),
        create_time_num: stamp.format("%Y%m%d%H%M%S").to_string().parse()?,
        employment_day: s.player.days,
        personal: initial,
        first_name: names[0][s.random(names[0].len())].clone(),
        last_name: names[1][s.random(names[1].len())].clone(),
        ..Default::default()
    };
    let mut x = character::Status::default();
    let jobs = initial_classes(s, &mut m, &mut x, false)?;
    let classes = initial_classes(s, &mut m, &mut x, true)?;
    initial_skills(&mut s.retail, &mut m, &jobs, &classes)?;
    m.hp = 100;
    m.mind = 100;
    m.reason = 100;
    let mut model = retail_maid(false);
    measure(&mut m, &model);
    stock_profile::initialize(&mut s.retail, &mut m, &model)?;
    m.cur_hp = m.a_hp().max(0);
    m.cur_mind = m.a_mind().max(0);
    model.set_up_model(&source.files, source.storage.as_ref(), "_I_default.menu")?;
    s.bodies.insert(id, mirror(&model));
    if let Some(r) = sub_row {
        apply_sub(s, id, &mut m, &mut x, &mut model, &sub, r)?;
    } else if let Some(r) = target {
        m.personal = personal.int(0, r);
        if personal.string(5, r) == "〇" {
            x.old = Some(Default::default());
        }
    }
    model.name_first = m.first_name.clone();
    model.name_last = m.last_name.clone();
    model.visible = true;
    s.retail.stock_models.insert(id, Arc::new(model));
    s.retail.characters.insert(m.guid_id.clone(), x);
    s.maids.insert(id, m);
    s.next_character = id.checked_add(1).context("character index overflow")?;
    s.stock.push(id);
    s.active_maids.resize(s.active_maids.len().max(1), None);
    s.active_maids[0] = Some(id);
    s.retail.stop_motions = true;
    s.requests.push(Request::CreateMaid(id));
    Ok(())
}

pub(super) fn create(s: &mut TjsState, args: &[Variant]) -> Result<Variant> {
    arity(args, 1)?;
    let mut pending = s.clone();
    initialize(&mut pending, &arg_string(args, 0))?;
    *s = pending;
    Ok(Variant::Void)
}

/// Stock alternatives of the retail BaseKagManager.TagCharaActivate, in native priority order.
pub(crate) fn select(s: &mut TjsState, args: &BTreeMap<String, String>) -> Result<Option<usize>> {
    if args.contains_key("leader") {
        return Ok(Some(
            s.stock
                .iter()
                .copied()
                .find(|i| s.maids[i].leader)
                .context("missing stock leader")?,
        ));
    }
    if args.contains_key("rank1") {
        let mut candidates: Vec<_> = s
            .stock
            .iter()
            .copied()
            .filter(|i| s.maids[i].popular_rank == 1)
            .collect();
        s.sort_maid_standard(&mut candidates);
        return Ok(candidates.first().copied());
    }
    if let Some(name) = args.get("main") {
        let t = s.retail.table("maid_status_personal_list.nei")?;
        let personal = t.int(0, named(&t, 1, name)?);
        return Ok(Some(
            s.stock
                .iter()
                .copied()
                .find(|id| {
                    s.retail.main_characters.contains(id) && s.maids[id].personal == personal
                })
                .with_context(|| format!("missing main character {name}"))?,
        ));
    }
    if let Some(name) = args.get("sub") {
        return Ok(s.stock.iter().copied().find(|id| {
            s.retail
                .characters
                .get(&s.maids[id].guid_id)
                .and_then(|x| x.sub_name.as_ref())
                == Some(name)
        }));
    }
    let slot = args
        .get("slot")
        .context("expected stock selector")?
        .parse::<usize>()?;
    Ok(Some(
        *s.stock
            .get(slot)
            .with_context(|| format!("missing stock index {slot}"))?,
    ))
}

pub(crate) fn edit_slider(s: &mut TjsState, id: usize, name: &str, fraction: f32) -> Result<()> {
    let source = s.retail.source()?.clone();
    let current = s
        .retail
        .stock_models
        .get(&id)
        .context("missing editor model")?;
    let mut model = (**current).clone();
    slide(&mut model, name, fraction)?;
    model.all_proc_prop(&source.files, source.storage.as_ref())?;
    measure(
        s.maids.get_mut(&id).context("missing editor status")?,
        &model,
    );
    s.bodies.insert(id, mirror(&model));
    s.retail.stock_models.insert(id, Arc::new(model));
    Ok(())
}

pub(crate) fn edit_wear(s: &mut TjsState, id: usize, file: &str) -> Result<()> {
    let (files, storage) = s.retail.editor_files()?;
    ensure!(s.retail.editor_owns(file), "item is not owned");
    let mut m = (**s
        .retail
        .stock_models
        .get(&id)
        .context("missing stock model")?)
    .clone();
    let item = crate::menu::MenuFile::parse(&files.read(storage.as_ref(), file)?)?;
    m.set_prop_file(&item.category, file, false)?;
    m.all_proc_prop(&files, storage.as_ref())?;
    let body = mirror(&m);
    s.bodies.insert(id, body);
    s.retail.stock_models.insert(id, Arc::new(m));
    Ok(())
}
