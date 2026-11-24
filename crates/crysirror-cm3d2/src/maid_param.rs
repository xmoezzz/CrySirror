// SPDX-License-Identifier: MPL-2.0
//! Operations of `MaidParam` and `PlayerParam` over [`crate::params`]: the
//! clamped setters, features/propensities from `feature_learn.nei`/
//! `propensity_learn.nei`, class acquisition and bonus status, sexual
//! experience, skills, the player's salon grade, items and trophies.
//!
//! Original quirks are kept: `AddFrustration` assigns instead of adding,
//! `AddTeachRate` discards its clamp, `SetSexualFront/Back` always end at 0.
use crate::{
    gamedata::{GameData, Skill},
    nei::Table,
    params::*,
};
use anyhow::{bail, Context, Result};

fn r3(v: i32) -> i32 {
    v.clamp(0, 999)
}
fn r4(v: i32) -> i32 {
    v.clamp(0, 9999)
}
fn r6(v: i64) -> i64 {
    v.clamp(0, 999_999)
}

/// `MaidParam.ConvertString`: no CR/LF/TAB/backslash, at least one space,
/// at most `max` text elements (scalar values here).
pub fn convert_string(s: &str, max: usize) -> String {
    let s: String = s.chars().filter(|c| !matches!(c, '\r' | '\n' | '\t' | '\\')).collect();
    let s = if s.is_empty() { " ".to_owned() } else { s };
    s.chars().take(max).collect()
}

/// `MaidParam.CsvDataBlock`: a rectangle of non-empty cells.
struct Block<'a> {
    t: &'a Table,
    left: usize,
    top: usize,
    right: i64,
    bottom: usize,
}

impl<'a> Block<'a> {
    fn new(t: &'a Table, start_y: usize) -> Self {
        let mut b = Block { t, left: 0, top: 0, right: 0, bottom: 0 };
        b.next_from(start_y);
        b
    }
    fn max_x(&self) -> i64 {
        self.right - self.left as i64 + 1
    }
    fn max_y(&self) -> i64 {
        self.bottom as i64 - self.top as i64 + 1
    }
    fn next_from(&mut self, start_y: usize) -> bool {
        let t = self.t;
        self.left = 0;
        self.bottom = t.rows.saturating_sub(1);
        self.right = 0;
        for i in start_y..t.rows {
            if t.exists(0, i) {
                self.top = i;
                let j = (0..t.columns).take_while(|j| t.exists(*j, i)).count() as i64;
                self.right = j - 1;
                break;
            }
        }
        if self.right <= 0 {
            (self.left, self.right, self.top, self.bottom) = (0, 0, 0, 0);
            return false;
        }
        for k in self.top..t.rows {
            if !(0..=self.right as usize).any(|l| t.exists(l, k)) {
                self.bottom = k - 1;
                break;
            }
        }
        for m in self.left..t.columns {
            for n in self.top..=self.bottom {
                if t.exists(m, n) && self.right < m as i64 - 1 {
                    self.right = m as i64 - 1;
                }
            }
        }
        true
    }
    fn next(&mut self) -> bool {
        let y = (self.top as i64 + self.max_y()).max(0) as usize;
        self.next_from(y)
    }
}

/// `MaidParam.IsStatusListCheck`.
fn status_list_check(st: &MaidStatus, list: &[(String, i32)]) -> Result<bool> {
    let value = |k: &str| -> Result<i32> {
        Ok(match k {
            "経験人数" => st.play_number,
            "淫欲" => st.a_inyoku(),
            "可憐" => st.a_lovely(),
            "M性" => st.a_m_value(),
            "気品" => st.a_elegance(),
            "変態" => st.a_hentai(),
            "魅惑" => st.a_charm(),
            "奉仕" => st.a_housi(),
            _ => bail!("不明なパラメータ : {k}"),
        })
    };
    for (k, v) in list {
        if k.contains(',') {
            let mut any = false;
            for p in k.split(',') {
                if *v <= value(p)? {
                    any = true;
                    break;
                }
            }
            if !any {
                return Ok(false);
            }
        } else if k == "特殊_過去の秘密" {
            if st.employment {
                return Ok(false);
            }
        } else if *v > value(k)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn measure_body(b: &mut crate::params::Body, prop: &dyn Fn(&str) -> f32, bust_adjustment: f32) {
        let f = prop;
        let num = 1340. + f("sintyou") * 4. + f("DouPer") * (1. + f("sintyou") * 0.005) + f("KubiScl") * 0.5 + f("HeadY") * 0.5;
        let n2 = 55. * f("RegFat") + 50. * f("sintyou") * 0.5 + 50. * f("DouPer") * 0.4;
        let n3 = 55. * f("RegMeet") + 50. * f("sintyou") * 0.5 + 50. * f("DouPer") * 0.4;
        let n4 = 10. * f("UdeScl") * 0.1;
        let n5 = 5. * f("ArmL") + 5. * f("sintyou") * 1. + 5. * f("UdeScl") * 0.5;
        let n6 = 70. * f("Hara") + 50. * f("sintyou") * 0.7 + 50. * f("west") * f("Hara") * 0.005;
        let n7 = 10. * f("MuneL") * 2.;
        let n8 = n7 * f("MuneTare") * 0.005;
        let n9 = 20. * f("west") * 0.5 + 15. * f("west") * f("sintyou") * 0.02 + 15. * f("west") * f("DouPer") * 0.01;
        let n10 = 10. * f("koshi") + 7. * f("koshi") * f("sintyou") * 0.04;
        let n11 = 4. * f("kata");
        let n12: f32 = 38000. + n2 + n3 + n4 + n5 + n6 + n7 + n8 + n9 + n10 + n11;

        b.height = (num / 10.) as i32;
        b.weight = (n12 / 1000.) as i32;
        let n13 = 70. + f("MuneL") * 0.31 + f("west") * 0.02 - bust_adjustment;
        b.bust = n13 as i32;
        b.waist = (40. + f("west") * 0.25 + f("Hara") * 0.35) as i32;
        b.hip = (65. + f("koshi") * 0.3 + f("RegFat") * 0.025 + f("RegMeet") * 0.025) as i32;
        b.cup = if n13 < 80. {
            "A".into()
        } else if 110. <= n13 {
            "N".into()
        } else {
            const CUPS: [char; 12] = ['B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M'];
            CUPS[(((n13 - 80.) / 2.5) as usize).min(11)].to_string()
        };
}

/// `MaidParam`: one maid's status with the game tables it reads.
pub struct MaidParam<'a> {
    pub st: &'a mut MaidStatus,
    pub data: &'a GameData,
}

impl MaidParam<'_> {
    /// `MaidParam.Initialize` after the C# field setup. `guid`/`now` come
    /// from the host (`Guid.NewGuid`, `DateTime.Now` in ja-JP "G" format).
    pub fn initialize(&mut self, player: &mut PlayerParam, guid: String, create_time: String, first: String, last: String) -> Result<()> {
        *self.st = MaidStatus::default();
        self.st.guid_id = guid;
        self.st.create_time_num = create_time.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0);
        self.st.create_time = create_time;
        self.st.employment_day = player.st.days;
        for c in 0..CLASS_SLOTS {
            self.st.maid_class[c].exp.set_list(self.data.maid_class_exp[c].clone());
            self.st.yotogi_class[c].exp.set_list(self.data.yotogi_class_exp[c].clone());
        }
        self.st.first_name = first;
        self.st.last_name = last;
        self.st.condition = 1;
        self.st.employment = false;
        self.st.leader = false;
        self.st.is_first_name_call = true;
        self.st.is_rental_maid = false;
        self.st.current_maid_class = 0;
        self.st.current_yotogi_class = 0;
        player.set_maid_class_open(0, true);
        player.set_yotogi_class_open(0, true);
        let bonus = self.data.trophy_bonus_maid_point(player.st.have_trophy.keys().copied());
        self.set_maid_point(bonus);
        self.st.hp = 100;
        self.st.cur_hp = 100;
        self.st.mind = 100;
        self.st.cur_mind = 100;
        self.st.reason = 100;
        self.st.cur_reason = 100;
        self.st.popular_rank = 0;
        self.update_feature()?;
        self.update_propensity()?;
        self.init_skill_data();
        self.init_class_types(player)?;
        Ok(())
    }

    pub fn set_employment(&mut self, v: bool, days: i32) {
        if v && !self.st.employment {
            self.st.employment_day = days;
        }
        self.st.employment = v;
    }
    pub fn set_last_name(&mut self, s: &str) {
        self.st.last_name = convert_string(s, 8);
    }
    pub fn set_first_name(&mut self, s: &str) {
        self.st.first_name = convert_string(s, 8);
    }
    pub fn set_free_comment(&mut self, s: &str) {
        self.st.free_comment = convert_string(s, 308);
    }
    pub fn set_maid_point(&mut self, v: i32) {
        self.st.maid_point = v.clamp(0, 999);
    }
    pub fn add_maid_point(&mut self, v: i32) {
        self.st.maid_point = (self.st.maid_point + v).clamp(0, 999);
    }
    pub fn set_feature(&mut self, f: i32, on: bool) -> bool {
        if on { self.st.feature.add(f) } else { self.st.feature.remove(&f).is_some() }
    }
    pub fn set_propensity(&mut self, p: i32, on: bool) -> bool {
        if on { self.st.propensity.add(p) } else { self.st.propensity.remove(&p).is_some() }
    }

    /// `MaidParam.UpdateBodyParam` from the maid's slider values.
    pub fn update_body(&mut self, prop: &dyn Fn(&str) -> f32) {
        measure_body(&mut self.st.body, prop, 0.);
    }

    /// `MaidParam.UpdateFeature`: the newly gained features.
    pub fn update_feature(&mut self) -> Result<Vec<i32>> {
        if !self.st.employment {
            self.st.feature.clear();
        }
        let before: Vec<i32> = self.st.feature.keys().copied().collect();
        self.update_learn(true, false)?;
        Ok(self.st.feature.keys().copied().filter(|f| !before.contains(f)).collect())
    }
    pub fn update_propensity(&mut self) -> Result<Vec<i32>> {
        if !self.st.employment {
            self.st.propensity.clear();
        }
        let before: Vec<i32> = self.st.propensity.keys().copied().collect();
        self.update_learn(false, true)?;
        Ok(self.st.propensity.keys().copied().filter(|f| !before.contains(f)).collect())
    }
    /// `MaidParam.IsGetPossiblePropensity`.
    pub fn is_get_possible_propensity(&mut self, p: i32) -> Result<bool> {
        if self.st.propensity.contains(&p) {
            return Ok(true);
        }
        let saved = self.st.propensity.clone();
        self.update_learn(false, true)?;
        let r = self.st.propensity.contains(&p);
        self.st.propensity = saved;
        Ok(r)
    }

    /// `MaidParam.UpdateFeatureAndPropensity`.
    fn update_learn(&mut self, feature: bool, propensity: bool) -> Result<()> {
        for (file, is_feature) in [("feature_learn.nei", true), ("propensity_learn.nei", false)] {
            if (is_feature && !feature) || (!is_feature && !propensity) {
                continue;
            }
            let t = self.data.table(file)?;
            let mut block = Block::new(t, 1);
            let n = match self.st.personal {
                0 => 1,
                1 => 2,
                2 => 3,
                _ => 0,
            };
            if n == 0 {
                continue;
            }
            for _ in 0..n - 1 {
                block.next();
            }
            for i in 0..block.max_y().max(0) as usize {
                let y = block.top + i;
                let name = t.string(block.left + 1, y);
                let names = if is_feature { FEATURE } else { PROPENSITY };
                let v = parse_enum(names, &name).with_context(|| format!("{file}: enum {name}"))?;
                let have = if is_feature { self.st.feature.contains(&v) } else { self.st.propensity.contains(&v) };
                if have || !self.learn_line(&block, y)? {
                    continue;
                }
                if is_feature { self.set_feature(v, true) } else { self.set_propensity(v, true) };
            }
        }
        Ok(())
    }

    /// The `line_call_back` of `UpdateFeatureAndPropensity`.
    fn learn_line(&self, b: &Block, y: usize) -> Result<bool> {
        let t = b.t;
        let need = t.string(b.left + 2, y);
        if need != "指定なし" {
            let ok = need.split(',').any(|k| match k {
                "処女" => self.st.seikeiken == 0,
                "前穴" => self.st.seikeiken == 1,
                "後穴" => self.st.seikeiken == 2,
                "両穴" => self.st.seikeiken == 3,
                _ => false,
            });
            if !ok {
                return Ok(false);
            }
        }
        let mut list = Vec::new();
        let mut l = b.left as i64 + 3;
        while l < b.max_x() {
            let x = (b.left as i64 + l) as usize;
            if t.exists(x, y) {
                let v = t.int(x + 1, y);
                if v > 0 {
                    list.push((t.string(x, y), v));
                }
            }
            l += 2;
        }
        status_list_check(self.st, &list)
    }

    /// `GetAcquisitionMaidClassTypeArray`/`…YotogiClassTypeArray`.
    pub fn acquisition(&self, yotogi: bool) -> Result<Vec<i32>> {
        let (file, names) = if yotogi { ("yotogi_class_acquisition_data.nei", YOTOGI_CLASS) } else { ("maid_class_acquisition_data.nei", MAID_CLASS) };
        let t = self.data.table(file)?;
        let b = Block::new(t, 0);
        let mut out = Vec::new();
        for i in 1..b.max_y().max(0) as usize {
            let y = b.top + i;
            let mut list = Vec::new();
            let mut x = b.left as i64 + 1;
            while x < b.max_x() {
                let cx = (b.left as i64 + x) as usize;
                let k = t.string(cx, y);
                if !k.is_empty() {
                    list.push((k, t.int(cx + 1, y)));
                }
                x += 2;
            }
            if status_list_check(self.st, &list)? {
                let name = t.string(b.left, y);
                out.push(parse_enum(names, &name).with_context(|| format!("{file}: class {name}"))?);
            }
        }
        Ok(out)
    }

    /// `UpdatetAcquisitionMaidClassType`/`…YotogiClassType`.
    pub fn update_acquisition(&mut self, player: &mut PlayerParam, yotogi: bool, set_have: bool) -> Result<Vec<i32>> {
        let mut out = Vec::new();
        for c in self.acquisition(yotogi)? {
            let d = if yotogi { &mut self.st.yotogi_class[c as usize] } else { &mut self.st.maid_class[c as usize] };
            if !d.is_have {
                if set_have {
                    d.is_have = true;
                    if yotogi { player.set_yotogi_class_open(c, true) } else { player.set_maid_class_open(c, true) }
                }
                out.push(c);
            }
        }
        Ok(out)
    }

    /// `MaidParam.InitMaidClassTypeAndYotogiClassType`.
    pub fn init_class_types(&mut self, player: &PlayerParam) -> Result<()> {
        if self.st.employment {
            return Ok(());
        }
        for yotogi in [false, true] {
            let current = if yotogi { self.st.current_yotogi_class } else { self.st.current_maid_class };
            let list = if yotogi { &mut self.st.yotogi_class } else { &mut self.st.maid_class };
            let mut had = Vec::new();
            for (i, c) in list.iter_mut().enumerate() {
                if c.is_have {
                    had.push(i as i32);
                }
                c.clear();
            }
            let acq = self.acquisition(yotogi)?;
            let list = if yotogi { &mut self.st.yotogi_class } else { &mut self.st.maid_class };
            let mut new = Vec::new();
            for c in &acq {
                let open = if yotogi { player.st.yotogi_class_open.contains(c) } else { player.st.maid_class_open.contains(c) };
                if open {
                    list[*c as usize].is_have = true;
                    if !had.contains(c) {
                        new.push(*c);
                    }
                }
            }
            let pick = if let Some(c) = new.last() {
                *c
            } else if list[current as usize].is_have {
                current
            } else if let Some(c) = acq.last() {
                *c
            } else {
                0
            };
            if yotogi { self.st.current_yotogi_class = pick } else { self.st.current_maid_class = pick }
        }
        self.update_class_bonus()
    }

    /// `MaidParam.UpdateMaidClassAndYotogiClassStatus`.
    pub fn update_class_bonus(&mut self) -> Result<()> {
        self.st.bonus = Bonus::default();
        for (file, yotogi) in [("maid_class_bonus_status.nei", false), ("yotogi_class_bonus_status.nei", true)] {
            let t = self.data.table(file)?;
            for y in 1..t.rows {
                if !t.exists(0, y) {
                    continue;
                }
                let name = t.string(0, y);
                let c = parse_enum(if yotogi { YOTOGI_CLASS } else { MAID_CLASS }, &name).with_context(|| format!("{file}: class {name}"))? as usize;
                let d = if yotogi { &self.st.yotogi_class[c] } else { &self.st.maid_class[c] };
                let level = if d.is_have { d.level() } else { 0 };
                for k in 1..=level.max(0) as usize {
                    if t.exists(k, y) {
                        apply_bonus(&mut self.st.bonus, &t.string(k, y))?;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn set_seikeiken_front(&mut self, v: bool) {
        let s = &mut self.st.seikeiken;
        if v {
            if *s != 3 && *s != 1 {
                *s = if *s == 0 { 1 } else { 3 };
            }
        } else if *s != 0 && *s != 2 {
            *s = if *s == 3 { 2 } else { 0 };
        }
    }
    pub fn set_seikeiken_back(&mut self, v: bool) {
        let s = &mut self.st.seikeiken;
        if v {
            if *s != 3 && *s != 2 {
                *s = if *s == 0 { 2 } else { 3 };
            }
        } else if *s != 0 && *s != 1 {
            *s = if *s == 3 { 1 } else { 0 };
        }
    }
    pub fn set_init_seikeiken(&mut self, v: i32) -> Result<()> {
        self.st.init_seikeiken = v;
        if !self.st.employment {
            self.update_init_play_number()?;
        }
        Ok(())
    }
    pub fn set_seikeiken(&mut self, v: i32) -> Result<()> {
        self.st.seikeiken = v;
        if !self.st.employment {
            self.set_init_seikeiken(v)?;
        }
        Ok(())
    }
    /// `MaidParam.UpdateInitPlayNumber`.
    pub fn update_init_play_number(&mut self) -> Result<()> {
        if self.st.employment {
            return Ok(());
        }
        self.set_play_number(0);
        if self.st.init_seikeiken == 0 {
            return Ok(());
        }
        let t = self.data.table("init_keiken_num_setting.nei")?;
        let mut y = 2;
        while y < t.rows && t.exists(0, y) {
            let s = match t.string(0, y).as_str() {
                "後穴" => 2,
                "両穴" => 3,
                _ => 1,
            };
            if s == self.st.init_seikeiken && t.int(2, y) <= self.st.a_inyoku() && t.int(3, y) <= self.st.a_hentai() {
                self.set_play_number(t.int(1, y));
                break;
            }
            y += 1;
        }
        Ok(())
    }

    pub fn set_play_number(&mut self, v: i32) {
        self.st.play_number = r4(v);
    }
    pub fn set_cur_hp(&mut self, v: i32) {
        self.st.cur_hp = v.clamp(0, self.st.a_hp());
    }
    pub fn set_cur_mind(&mut self, v: i32) {
        self.st.cur_mind = v.clamp(0, self.st.a_mind());
    }
    pub fn set_cur_reason(&mut self, v: i32) {
        self.st.cur_reason = v.clamp(0, self.st.a_reason());
    }
    pub fn set_popular_rank(&mut self, v: i32) {
        self.st.popular_rank = v.clamp(0, 99);
    }
    pub fn set_evaluation(&mut self, v: i64) {
        self.st.evaluation = r6(v);
    }
    pub fn add_evaluation(&mut self, v: i64, ranking: bool) {
        if ranking {
            self.st.evaluation = r6(self.st.evaluation + v);
        }
        self.st.total_evaluation = r6(self.st.total_evaluation + v);
    }
    pub fn set_sales(&mut self, v: i64) {
        self.st.sales = v.clamp(0, MONEY_MAX);
    }
    pub fn add_sales(&mut self, v: i64, ranking: bool) {
        if ranking {
            self.st.sales = (self.st.sales + v).clamp(0, MONEY_MAX);
        }
        self.st.total_sales = (self.st.total_sales + v).clamp(0, MONEY_MAX);
    }

    /// The `Add*` setters addressed by `TJSFuncAddMaidStatus`'s keys.
    pub fn add_status(&mut self, key: &str, v: i32) -> bool {
        let st = &mut *self.st;
        match key {
            "夜伽プレイ回数" => st.yotogi_play_count = r4(st.yotogi_play_count + v),
            "接客プレイ回数" => st.others_play_count = r4(st.others_play_count + v),
            "好感度" => st.likability = r3(st.likability + v),
            "習得率" => st.study_rate = (st.study_rate + v).clamp(0, 1000),
            "接待" => st.reception = r4(st.reception + v),
            "お世話" => st.care = r4(st.care + v),
            "可憐" => st.lovely = r4(st.lovely + v),
            "淫欲" => st.inyoku = r4(st.inyoku + v),
            "気品" => st.elegance = r4(st.elegance + v),
            "M性" => st.m_value = r4(st.m_value + v),
            "魅惑" => st.charm = r4(st.charm + v),
            "変態" => st.hentai = r4(st.hentai + v),
            "奉仕" => st.housi = r4(st.housi + v),
            // AddTeachRate: the clamp result is discarded in the original.
            "指導" => st.teach_rate += v,
            "経験人数" => st.play_number = r4(st.play_number + v),
            // AddFrustration assigns the argument in the original.
            "欲求不満" => st.frustration = v.clamp(0, 100),
            "体力" => st.hp = r3(st.hp + v),
            "精神力" => st.mind = r3(st.mind + v),
            "理性" => st.reason = r3(st.reason + v),
            "興奮値" => st.cur_excite = (st.cur_excite + v).clamp(-100, 300),
            "現在精神力" => st.cur_mind = r3(st.cur_mind + v),
            "現在理性" => st.cur_reason = r3(st.cur_reason + v),
            _ => return false,
        }
        true
    }

    /// `AddSexual*(add, limit)`; front/back always end at 0 in the original.
    pub fn add_sexual(&mut self, part: &str, v: i32, limit: i32) {
        let s = &mut self.st.sexual;
        match part {
            "口" => s.mouth = (s.mouth + v).clamp(0, limit),
            "喉" => s.throat = (s.throat + v).clamp(0, limit),
            "乳首" => s.nipple = (s.nipple + v).clamp(0, limit),
            "前穴" => s.front = 0,
            "後穴" => s.back = 0,
            "クリ" => s.curi = (s.curi + v).clamp(0, limit),
            _ => {}
        }
    }

    /// `Yotogi.SkillData.IsCheckGetSkill`.
    pub fn is_check_get_skill(&self, s: &Skill, seikeiken_check: bool, class_check: bool) -> bool {
        let g = &s.get;
        if !class_check && (g.maid_class_level >= 1 || g.yotogi_class_level >= 1) {
            return false;
        }
        if seikeiken_check && !g.seikeiken.get(self.st.seikeiken as usize).copied().unwrap_or(false) {
            return false;
        }
        let st = &*self.st;
        if st.a_inyoku() < g.inyoku
            || st.a_m_value() < g.m_value
            || st.a_hentai() < g.hentai
            || st.a_housi() < g.housi
            || st.a_lovely() < g.lovely
            || st.a_elegance() < g.elegance
            || st.a_charm() < g.charm
        {
            return false;
        }
        if g.maid_class_level > 0 {
            if st.current_maid_class != g.maid_class || st.maid_class[g.maid_class as usize].level() < g.maid_class_level {
                return false;
            }
        } else if g.yotogi_class_level > 0 && (st.current_yotogi_class != g.yotogi_class || st.yotogi_class[g.yotogi_class as usize].level() < g.yotogi_class_level) {
            return false;
        }
        true
    }

    /// `MaidParam.SetNewGetSkill`.
    pub fn set_new_get_skill(&mut self, id: i32) -> bool {
        if self.st.skill_data.contains(&id) {
            return false;
        }
        let mut s = SkillData { id, ..Default::default() };
        if let Some(d) = self.data.skill(id) {
            s.exp.set_list(d.exp_table.clone());
        }
        self.st.skill_data.insert(id, s);
        true
    }

    /// `MaidParam.InitSkillData`.
    pub fn init_skill_data(&mut self) {
        self.st.skill_data.clear();
        let data = self.data;
        for cat in &data.skills {
            for (id, s) in cat {
                if self.is_check_get_skill(s, false, false) {
                    self.set_new_get_skill(*id);
                }
            }
        }
    }

    /// Need tables of loaded skills (`Status.Deserialize`).
    pub fn restore_skill_tables(&mut self) {
        let data = self.data;
        for s in self.st.skill_data.values_mut() {
            if let Some(d) = data.skill(s.id) {
                s.exp.need = d.exp_table.clone();
            }
        }
        for c in 0..CLASS_SLOTS {
            self.st.maid_class[c].exp.need = data.maid_class_exp[c].clone();
            self.st.yotogi_class[c].exp.need = data.yotogi_class_exp[c].clone();
        }
    }

    pub fn set_new_get_work(&mut self, id: i32) -> bool {
        if self.st.work_data.contains(&id) {
            return false;
        }
        self.st.work_data.insert(id, WorkData { id, play_count: 0, level: 1 });
        true
    }

    /// `MaidParam.Serialize`.
    pub fn write(st: &MaidStatus, w: &mut crate::binio::Writer) {
        w.str("CM3D2_MAID_PPARAM");
        w.i32(100);
        st.write(w);
        w.i32(1_923_480_616);
    }
    /// `MaidParam.Deserialize`.
    pub fn read(&mut self, r: &mut crate::binio::Reader) -> Result<()> {
        r.expect_str("CM3D2_MAID_PPARAM")?;
        let version = r.i32()?;
        self.st.read(r, version)?;
        anyhow::ensure!(r.i32()? == 1_923_480_616, "メイドパラメータのロードに失敗しました");
        self.restore_skill_tables();
        Ok(())
    }
}

/// `UpdateMaidClassAndYotogiClassStatus`'s `体力+5,精神力-3` cells.
fn apply_bonus(b: &mut Bonus, cell: &str) -> Result<()> {
    for tok in cell.split(',') {
        let (k, v) = if let Some((k, v)) = tok.split_once('+') {
            (k, v.trim().parse::<i32>().with_context(|| format!("bonus {tok}"))?)
        } else if let Some((k, v)) = tok.split_once('-') {
            (k, -v.trim().parse::<i32>().with_context(|| format!("bonus {tok}"))?)
        } else {
            bail!("[{tok}] 記述が不正です");
        };
        let f = match k {
            "体力" => &mut b.hp,
            "精神力" => &mut b.mind,
            "接待" => &mut b.reception,
            "お世話" => &mut b.care,
            "可憐" => &mut b.lovely,
            "淫欲" => &mut b.inyoku,
            "気品" => &mut b.elegance,
            "M性" => &mut b.m_value,
            "魅惑" => &mut b.charm,
            "変態" => &mut b.hentai,
            "奉仕" => &mut b.housi,
            "指導" => &mut b.teach_rate,
            _ => bail!("{k}には未対応です"),
        };
        *f += v;
    }
    Ok(())
}

/// `PlayerParam`.
pub struct PlayerParam<'a> {
    pub st: &'a mut PlayerStatus,
    pub data: &'a GameData,
}

impl PlayerParam<'_> {
    /// `PlayerParam()` after the field setup: item list and salon grade.
    pub fn init(&mut self) {
        *self.st = PlayerStatus::default();
        self.create_have_item_list();
        self.check_salon_grade_level_up();
    }

    /// `PlayerParam.CreateHaveItemList`.
    pub fn create_have_item_list(&mut self) {
        let d = self.data;
        for (_, item) in d.shop.iter() {
            if item.need_ids.is_some() {
                continue;
            }
            for m in &item.menus {
                if !self.st.have_item.contains(m.as_str()) {
                    self.st.have_item.insert(m.clone(), false);
                }
            }
        }
        for (menus, _) in &d.set_cards {
            for m in menus {
                if !self.st.have_item.contains(m.as_str()) {
                    self.st.have_item.insert(m.clone(), false);
                }
            }
        }
        for m in &d.scenario_items {
            if !self.st.have_item.contains(m.as_str()) {
                self.st.have_item.insert(m.clone(), false);
            }
        }
    }

    /// `PlayerParam.AddHaveItem`: true when newly owned.
    pub fn add_have_item(&mut self, name: &str) -> Result<bool> {
        let name = name.to_lowercase();
        let had = *self.st.have_item.get(name.as_str()).with_context(|| format!("{name}の取得設定を変える事はできません"))?;
        self.st.have_item.insert(name, true);
        Ok(!had)
    }

    pub fn set_name(&mut self, s: &str) {
        self.st.player_name = convert_string(s, 8);
    }
    pub fn set_scenario_phase(&mut self, v: i32) {
        self.st.scenario_phase = v.clamp(0, 2);
        self.st.phase_days = 0;
    }
    pub fn add_days(&mut self, v: i32) {
        self.st.days = (self.st.days + v).max(0);
    }
    pub fn add_phase_days(&mut self, v: i32) {
        self.st.phase_days = (self.st.phase_days + v).max(0);
    }
    pub fn add_money(&mut self, v: i64) {
        self.st.money = (self.st.money + v).clamp(0, MONEY_MAX);
    }
    pub fn add_shop_use_money(&mut self, v: i64) {
        self.st.shop_use_money = (self.st.shop_use_money + v).clamp(0, MONEY_MAX);
    }
    pub fn set_salon_loan(&mut self, v: i64) {
        self.st.salon_loan = v.clamp(-MONEY_MAX, 0);
    }
    pub fn add_salon_loan(&mut self, v: i64) {
        self.st.salon_loan = (self.st.salon_loan + v).clamp(-MONEY_MAX, 0);
    }
    pub fn set_init_salon_loan(&mut self, v: i64) {
        self.st.init_salon_loan = v.clamp(-MONEY_MAX, 0);
    }
    pub fn add_salon_clean(&mut self, v: i32) {
        self.st.salon_clean = r3(self.st.salon_clean + v);
    }
    pub fn add_salon_beautiful(&mut self, v: i32) {
        self.st.salon_beautiful = r3(self.st.salon_beautiful + v);
    }
    pub fn add_salon_evaluation(&mut self, v: i32) {
        self.st.salon_evaluation = r3(self.st.salon_evaluation + v);
    }

    pub fn set_salon_grade(&mut self, g: i32) {
        self.st.current_salon_grade = g.clamp(1, SALON_GRADE_MAX);
        if self.st.best_salon_grade < self.st.current_salon_grade {
            self.st.best_salon_grade = self.st.current_salon_grade;
        }
        self.check_salon_grade_shop_lineup();
    }
    /// `PlayerParam.CheckSalonGradeLevelUp`.
    pub fn check_salon_grade_level_up(&mut self) -> bool {
        let s = &self.st;
        let exp = s.salon_evaluation * (1000 + s.salon_clean + s.salon_beautiful) / 500 + (s.salon_clean + s.salon_beautiful);
        let mut g = 0;
        for (i, (need, _)) in self.data.salon_grades.iter().enumerate() {
            if *need <= exp {
                g = i as i32 + 1;
            }
        }
        let up = self.st.best_salon_grade < g;
        if g < self.st.best_salon_grade {
            g = self.st.best_salon_grade;
        }
        self.set_salon_grade(g);
        up
    }
    pub fn check_salon_grade_shop_lineup(&mut self) {
        let d = self.data;
        for i in 0..(self.st.current_salon_grade.max(0) as usize).min(d.salon_grades.len()) {
            for id in &d.salon_grades[i].1 {
                if !self.st.shop_lineup.contains(id) {
                    self.st.shop_lineup.insert(*id, SHOP_NEW);
                }
            }
        }
    }
    pub fn set_shop_lineup(&mut self, id: i32, status: i32) -> bool {
        let new = !self.st.shop_lineup.contains(&id);
        self.st.shop_lineup.insert(id, status);
        new
    }
    pub fn set_flag(&mut self, k: &str, v: i32) {
        self.st.generic_flag.insert(k.to_owned(), v);
    }

    /// `PlayerParam.AddHaveTrophy`: true when newly owned (the caller shows
    /// `TrophyAchieveEffect`).
    pub fn add_have_trophy(&mut self, id: i32) -> bool {
        let d = self.data;
        let Some(t) = d.trophies.get(&id) else { return false };
        let new = self.st.have_trophy.add(id);
        for l in &t.lineup {
            if !self.st.is_shop_lineup(*l) {
                self.set_shop_lineup(*l, SHOP_NEW);
            }
        }
        new
    }

    /// `PlayerParam.CheckTrophyContractType` over the stock maids' contract
    /// types; returns newly owned trophies.
    pub fn check_trophy_contract_type(&mut self, contracts: impl Iterator<Item = i32>) -> Vec<i32> {
        if [5, 10, 20, 30, 40, 50].iter().all(|i| self.st.have_trophy.contains(i)) {
            return Vec::new();
        }
        let (mut free, mut excl) = (0, 0);
        for c in contracts {
            match c {
                1 => free += 1,
                2 => excl += 1,
                _ => {}
            }
        }
        let mut got = Vec::new();
        for (n, need, id) in [(free, 1, 5), (free, 3, 10), (free, 5, 20), (excl, 1, 30), (excl, 3, 40), (excl, 5, 50)] {
            if need <= n && self.add_have_trophy(id) {
                got.push(id);
            }
        }
        got
    }

    pub fn set_maid_class_open(&mut self, c: i32, on: bool) {
        if on {
            self.st.maid_class_open.add(c);
        } else {
            self.st.maid_class_open.remove(&c);
        }
    }
    pub fn set_yotogi_class_open(&mut self, c: i32, on: bool) {
        if on {
            self.st.yotogi_class_open.add(c);
        } else {
            self.st.yotogi_class_open.remove(&c);
        }
    }

    pub fn is_available_shop(&self) -> bool {
        2 <= self.st.scenario_phase
    }
    pub fn is_available_ranking(&self) -> bool {
        self.st.flag("__ランキング解放") == 1
    }
    pub fn is_available_competitive_show(&self) -> bool {
        2 <= self.st.scenario_phase || self.st.flag("品評会フラグ") == 1
    }
    pub fn is_available_employment(&self) -> bool {
        2 <= self.st.scenario_phase
    }
    pub fn is_available_class_change(&self) -> bool {
        10 <= self.st.days
    }
    pub fn is_available_repayment(&self) -> bool {
        1 <= self.st.scenario_phase
    }

    /// `PlayerParam.Serialize`.
    pub fn write(st: &PlayerStatus, w: &mut crate::binio::Writer) {
        w.str("CM3D2_PPARAM");
        w.i32(100);
        st.write(w);
        w.i32(348_195_810);
    }
    /// `PlayerParam.Deserialize` (then `CreateHaveItemList`).
    pub fn read(&mut self, r: &mut crate::binio::Reader) -> Result<()> {
        r.expect_str("CM3D2_PPARAM")?;
        let version = r.i32()?;
        self.st.read(r, version)?;
        anyhow::ensure!(r.i32()? == 348_195_810, "主人公パラメータのロードに失敗しました");
        self.create_have_item_list();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seikeiken_transitions_follow_the_original() {
        let data = GameData::from_tables_for_test();
        let mut st = MaidStatus::default();
        let mut p = MaidParam { st: &mut st, data: &data };
        p.set_seikeiken_front(true);
        assert_eq!(p.st.seikeiken, 1);
        p.set_seikeiken_back(true);
        assert_eq!(p.st.seikeiken, 3);
        p.set_seikeiken_front(false);
        assert_eq!(p.st.seikeiken, 2);
        assert!(p.add_status("欲求不満", 30));
        assert!(p.add_status("欲求不満", 20));
        assert_eq!(p.st.frustration, 20);
    }

    #[test]
    fn bonus_cells_add_and_subtract() {
        let mut b = Bonus::default();
        apply_bonus(&mut b, "体力+5,精神力-3").unwrap();
        assert_eq!((b.hp, b.mind), (5, -3));
        assert!(apply_bonus(&mut b, "体力5").is_err());
    }

    #[test]
    fn convert_string_strips_and_limits() {
        assert_eq!(convert_string("a\tb\\c", 8), "abc");
        assert_eq!(convert_string("", 8), " ");
        assert_eq!(convert_string("一二三四五六七八九", 8).chars().count(), 8);
    }
}
