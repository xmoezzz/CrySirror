// SPDX-License-Identifier: MPL-2.0
//! The TJS2 context of `ScriptManager` (`DLL_TJS_*` over the plug-in's
//! embedded TJS2), with every native function `ScriptManager.Initialize`
//! registers through `TJSScript.AddFunction`.
//!
//! Maid and player parameters live in [`TjsState`] (owned by the TJS host).
//! Character-manager state the functions read (active slots, stock order,
//! body props/slots, the background) is mirrored by the game each frame;
//! calls that change characters (`CreateNewMaid`, `SetActiveSlot…`) queue a
//! [`Request`] that the game executes after the script step.
use crate::{
    gamedata::GameData,
    maid_param::{MaidParam, PlayerParam},
    params::*,
};
use anyhow::{Context, Result};
use crysirror_tjs2::runtime::{ObjectHandle, Runtime, TjsHost, Variant};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

/// What `ScriptManager`'s functions read of a `Maid`'s `TBody`.
#[derive(Clone, Debug, Default)]
pub struct BodyMirror {
    /// MPN name → (`MaidProp.type`, value).
    pub props: HashMap<String, (u8, i32)>,
    pub menu_files: HashMap<String, String>,
    /// `TBody.SlotID` names with a loaded model.
    pub loaded_slots: HashSet<String>,
    /// `TBodySkin.GetParam` tables in slot order.
    pub params: Vec<BTreeMap<String, String>>,
}

/// Character-manager work queued by a TJS function.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// `CharacterMgr.AddStockMaid` + `SetActiveMaid(maid, 0)` + visible;
    /// the parameters are already initialized at this character index.
    CreateMaid(usize),
    DeactivateMaid(usize),
    RefreshMaid(usize),
    /// `ScriptManager.StopMotionScript` then `SetActiveMaid(chara, slot)`.
    SetActiveMaid { slot: usize, chara: usize, visible: bool },
    /// `ScriptManager.ClearCallStack`.
    ClearCallStack,
    /// `TrophyAchieveEffect.EffectStart`.
    TrophyEffect(i32),
}

/// Game parameters reached by `ScriptManager`'s TJS functions.
#[derive(Clone, Default)]
pub struct TjsState {
    pub data: Option<Arc<GameData>>,
    pub player: PlayerStatus,
    pub(crate) retail: retail::State,
    /// `MaidParam` per character index.
    pub maids: BTreeMap<usize, MaidStatus>,
    /// `CharacterMgr` active maid slots → character index (mirrored).
    pub active_maids: Vec<Option<usize>>,
    /// Stock maids (non-NPC) in `m_listStockMaid` order (mirrored).
    pub stock: Vec<usize>,
    /// Index the next created character gets (mirrored).
    pub next_character: usize,
    pub bodies: HashMap<usize, BodyMirror>,
    /// `BgMgr.GetBGName` (mirrored).
    pub bg_name: String,
    /// Standalone PlayerStatus.Status.clubGauge.
    pub club_gauge: i32,
    /// CMSystem.m_SystemVers, distinct from per-player and temporary flags.
    pub system_versions: BTreeMap<String, String>,
    /// `YotogiStageSelectManager.StageName`.
    pub stage_name: String,
    /// `YotogiStageSelectManager.StagePrefab`.
    pub stage_prefab: String,
    /// `CMSystem` temporary flags (`SetTmpFlag`/`GetTmpFlag`).
    pub tmp_flags: BTreeMap<String, i64>,
    /// `ScriptManager.stock_maid_loop_list`.
    pub stock_loop: Vec<usize>,
    /// `SceneCharacterSelect.chara_guid_stock_list`.
    pub chara_guid_list: Vec<String>,
    pub requests: Vec<Request>,
    rng: u64,
}

impl TjsState {
    pub fn maid(&self, slot: usize) -> Option<&MaidStatus> {
        let i = (*self.active_maids.get(slot)?)?;
        self.maids.get(&i)
    }
    pub fn maid_mut(&mut self, slot: usize) -> Option<&mut MaidStatus> {
        let i = (*self.active_maids.get(slot)?)?;
        Some(self.maids.entry(i).or_default())
    }
    pub fn data(&self) -> Result<Arc<GameData>> {
        self.data.clone().context("CM3D2 game tables are not loaded")
    }
    /// `UnityEngine.Random.Range(0, n)` (engine-owned generator).
    pub fn random(&mut self, n: usize) -> usize {
        if self.rng == 0 {
            self.rng = 0x9e37_79b9_7f4a_7c15;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        if n == 0 { 0 } else { (self.rng % n as u64) as usize }
    }
    pub fn seed(&mut self, seed: u64) {
        self.rng = seed | 1;
    }
    pub(crate) fn guid(&mut self) -> String {
        let mut b = [0u8; 16];
        for x in &mut b {
            *x = self.random(256) as u8;
        }
        b[6] = (b[6] & 0x0f) | 0x40;
        b[8] = (b[8] & 0x3f) | 0x80;
        let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
        format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
    }

    /// `Maid.Initialize` → `MaidParam.Initialize` for character `index`
    /// (`npc_name`: `Param.SetName(string.Empty, name)` of LoadDefault).
    pub fn init_maid(&mut self, index: usize, npc_name: Option<&str>, create_time: String) -> Result<()> {
        let data = self.data()?;
        let pick = |s: &mut Self, list: &[String]| -> String { if list.is_empty() { String::new() } else { list[s.random(list.len())].clone() } };
        let first = pick(self, &data.random_names[1]);
        let last = pick(self, &data.random_names[0]);
        let guid = self.guid();
        let mut m = MaidStatus::default();
        {
            let mut player = PlayerParam { st: &mut self.player, data: &data };
            MaidParam { st: &mut m, data: &data }.initialize(&mut player, guid, create_time, first, last)?;
        }
        if let Some(name) = npc_name {
            let mut p = MaidParam { st: &mut m, data: &data };
            p.set_last_name("");
            p.set_first_name(name);
        }
        self.maids.insert(index, m);
        Ok(())
    }

    /// `ScheduleAPI` maids of the schedule slots whose work type matches.
    pub(crate) fn schedule_maids(&self, noon: bool, types: &[i32]) -> Vec<usize> {
        let Some(data) = &self.data else { return Vec::new() };
        let mut out = Vec::new();
        for slot in &self.player.schedule_slot {
            if slot.maid_guid.is_empty() {
                continue;
            }
            let Some(i) = self.stock.iter().copied().find(|i| self.maids.get(i).is_some_and(|m| m.guid_id == slot.maid_guid)) else { continue };
            let m = &self.maids[&i];
            let t = if noon { data.schedule.noon_type(m.noon_work_id) } else { data.schedule.night_type(m.night_work_id) };
            if t.is_some_and(|t| types.contains(&t)) {
                out.push(i);
            }
        }
        out
    }

    /// `CharacterSelectManager.SortMaidStandard`.
    pub fn sort_maid_standard(&self, list: &mut [usize]) {
        let slot = |i: &usize| -> i32 {
            let guid = self.maids.get(i).map(|m| m.guid_id.as_str()).unwrap_or("");
            self.player.schedule_slot.iter().position(|s| !s.maid_guid.is_empty() && s.maid_guid == guid).map_or(-1, |p| p as i32)
        };
        list.sort_by(|a, b| {
            let (ma, mb) = (self.maids.get(a).cloned().unwrap_or_default(), self.maids.get(b).cloned().unwrap_or_default());
            use std::cmp::Ordering::*;
            if ma.leader != mb.leader {
                return if ma.leader { Less } else { Greater };
            }
            let (sa, sb) = (slot(a), slot(b));
            if (sa == -1) != (sb == -1) {
                return if sa != -1 { Less } else { Greater };
            }
            sa.cmp(&sb).then(ma.create_time_num.cmp(&mb.create_time_num)).then_with(|| mb.guid_id.cmp(&ma.guid_id))
        });
    }
}

pub struct Host {
    pub state: TjsState,
    /// `tjs.arc` sources by lower-case name.
    pub sources: HashMap<String, String>,
}

impl TjsHost for Host {
    fn read_text(&mut self, name: &str, _mode: &str) -> crysirror_tjs2::Result<String> {
        self.sources
            .get(&name.to_ascii_lowercase())
            .cloned()
            .ok_or_else(|| crysirror_tjs2::TjsError::runtime(format!("TJS storage {name} not found")))
    }
}

pub struct Tjs {
    pub runtime: Runtime<Host>,
}

fn arg_string(args: &[Variant], i: usize) -> String {
    match args.get(i) {
        Some(Variant::String(s)) => s.clone(),
        Some(Variant::Integer(n)) => n.to_string(),
        Some(Variant::Real(r)) => r.to_string(),
        _ => String::new(),
    }
}
fn arg_int(args: &[Variant], i: usize) -> i64 {
    match args.get(i) {
        Some(Variant::Integer(n)) => *n,
        Some(Variant::Real(r)) => *r as i64,
        Some(Variant::String(s)) => crate::text::atoi(s.as_bytes()) as i64,
        _ => 0,
    }
}
/// `TJSVariantRef.AsBool`.
fn arg_bool(args: &[Variant], i: usize) -> bool {
    match args.get(i) {
        Some(Variant::String(s)) => !s.is_empty() && s != "0" && !s.eq_ignore_ascii_case("false"),
        Some(Variant::Real(r)) => *r != 0.,
        _ => arg_int(args, i) != 0,
    }
}
fn int(v: impl Into<i64>) -> Variant {
    Variant::Integer(v.into())
}
fn boolean(v: bool) -> Variant {
    Variant::Integer(i64::from(v))
}

type Func = fn(&mut TjsState, &[Variant]) -> Result<Variant>;

fn maid<'a>(s: &'a mut TjsState, a: &[Variant]) -> Result<&'a mut MaidStatus> {
    let slot = arg_int(a, 0) as usize;
    s.maid_mut(slot).with_context(|| format!("no maid in active slot {slot}"))
}

/// `GetMaidStatus`'s keys.
fn maid_status(m: &MaidStatus, key: &str) -> Option<Variant> {
    let text = |s: String| Some(Variant::String(s));
    Some(match key {
        "性経験" => return text(enum_text(SEIKEIKEN_TEXT, m.seikeiken)),
        "初期性経験" => return text(enum_text(SEIKEIKEN_TEXT, m.init_seikeiken)),
        "性格" => return text(enum_text(PERSONAL_TEXT, m.personal)),
        "契約タイプ" => return text(enum_text(CONTRACT_TEXT, m.contract_type)),
        "メイドクラス" => return text(enum_text(MAID_CLASS_TEXT, m.current_maid_class)),
        "夜伽クラス" => return text(enum_text(YOTOGI_CLASS_TEXT, m.current_yotogi_class)),
        "状態" => return text(enum_text(CONDITION_TEXT, m.condition)),
        "特殊状態" => return text(enum_text(CONDITION_SPECIAL_TEXT, m.condition_special)),
        "カップ" => return text(m.body.cup.clone()),
        "苗字" => return text(m.last_name.clone()),
        "名前" => return text(m.first_name.clone()),
        "夜伽プレイ回数" => int(m.yotogi_play_count),
        "接客プレイ回数" => int(m.others_play_count),
        "興奮値" => int(m.a_cur_excite()),
        "好感度" => int(m.likability),
        "習得率" => int(m.a_study_rate()),
        "接待" => int(m.a_reception()),
        "お世話" => int(m.a_care()),
        "可憐" => int(m.a_lovely()),
        "淫欲" => int(m.a_inyoku()),
        "気品" => int(m.a_elegance()),
        "M性" => int(m.a_m_value()),
        "魅惑" => int(m.a_charm()),
        "変態" => int(m.a_hentai()),
        "奉仕" => int(m.a_housi()),
        "指導" => int(m.a_teach_rate()),
        "経験人数" => int(m.play_number),
        "欲求不満" => int(m.a_frustration()),
        "淫乱ギャップ値" => int(m.inran_gap()),
        "M性ギャップ値" => int(m.m_value_gap()),
        "変態ギャップ値" => int(m.hentai_gap()),
        "昼仕事ID" => int(m.noon_work_id),
        "夜仕事ID" => int(m.night_work_id),
        "雇用中" => boolean(m.employment),
        "昼仕事レベル" => int(m.work_data.get(&m.noon_work_id).map_or(1, |w| w.level)),
        "夜伽クラスレベル" => int(m.yotogi_class.get(m.current_yotogi_class as usize).map_or(0, |c| c.level())),
        "メイドクラスレベル" => int(m.maid_class.get(m.current_maid_class as usize).map_or(0, |c| c.level())),
        _ => return None,
    })
}

const FUNCS: &[(&str, Func)] = &[
    ("GetMaidMPN", |s, a| {
        let slot = arg_int(a, 0) as usize;
        let i = s.active_maids.get(slot).copied().flatten().with_context(|| format!("no maid in slot {slot}"))?;
        let mpn = arg_string(a, 1);
        let body = s.bodies.get(&i).context("maid body is not loaded")?;
        let (kind, value) = *body.props.get(&mpn).with_context(|| format!("MPN {mpn}"))?;
        Ok(if kind == 3 { Variant::String(mpn) } else { int(value) })
    }),
    ("SetMaidFlag", |s, a| {
        let (slot, o) = if a.len() == 3 { (arg_int(a, 0) as usize, 1) } else { (0, 0) };
        let v = arg_int(a, o + 1) as i32;
        let name = arg_string(a, o);
        s.maid_mut(slot).with_context(|| format!("no maid {slot}"))?.generic_flag.insert(name, v);
        Ok(int(v))
    }),
    ("SetManFlag", |s, a| {
        let o = usize::from(a.len() == 3);
        let v = arg_int(a, o + 1) as i32;
        s.player.generic_flag.insert(arg_string(a, o), v);
        Ok(int(v))
    }),
    ("SetTmpFlag", |s, a| {
        let v = arg_int(a, 1) as i32;
        s.tmp_flags.insert(arg_string(a, 0), v.into());
        Ok(int(v))
    }),
    ("GetMaidFlag", |s, a| {
        let (slot, o) = if a.len() == 2 { (arg_int(a, 0) as usize, 1) } else { (0, 0) };
        let m = s.maid(slot).with_context(|| format!("no maid {slot}"))?;
        Ok(int(m.flag(&arg_string(a, o))))
    }),
    ("GetManFlag", |s, a| {
        let o = usize::from(a.len() == 2);
        Ok(int(s.player.flag(&arg_string(a, o))))
    }),
    ("GetTmpFlag", |s, a| Ok(int(s.tmp_flags.get(&arg_string(a, 0)).copied().unwrap_or(0)))),
    ("GetMaidStatus", |s, a| Ok(maid_status(maid(s, a)?, &arg_string(a, 1)).unwrap_or(Variant::Void))),
    ("IsSeikantai", |s, a| {
        let x = maid(s, a)?.sexual;
        let v = match arg_string(a, 1).as_str() {
            "口" => x.mouth,
            "喉" => x.throat,
            "乳首" => x.nipple,
            "前穴" => x.front,
            "後穴" => x.back,
            "クリ" => x.curi,
            _ => return Ok(Variant::Void),
        };
        Ok(boolean(1000 <= v))
    }),
    ("CreateNewMaid", |s, _| {
        let index = s.next_character;
        let time = s.maids.values().next().map(|m| m.create_time.clone()).unwrap_or_default();
        s.init_maid(index, None, time)?;
        s.next_character += 1;
        s.stock.push(index);
        s.active_maids.resize(s.active_maids.len().max(1), None);
        s.active_maids[0] = Some(index);
        s.requests.push(Request::CreateMaid(index));
        Ok(Variant::Void)
    }),
    ("AddMaidStatus", |s, a| {
        let data = s.data()?;
        let key = arg_string(a, 1);
        let v = arg_int(a, 2) as i32;
        MaidParam { st: maid(s, a)?, data: &data }.add_status(&key, v);
        Ok(Variant::Void)
    }),
    ("IsMaidFeature", |s, a| {
        let Some(f) = FEATURE.iter().position(|n| *n == arg_string(a, 1)) else { return Ok(Variant::Void) };
        Ok(boolean(maid(s, a)?.is_feature(f as i32)))
    }),
    ("IsMaidPropensity", |s, a| {
        let Some(p) = PROPENSITY.iter().position(|n| *n == arg_string(a, 1)) else { return Ok(Variant::Void) };
        Ok(boolean(maid(s, a)?.is_propensity(p as i32)))
    }),
    ("GetYotogiSkillLevel", |s, a| {
        let id = arg_int(a, 1) as i32;
        Ok(int(maid(s, a)?.skill_data.get(&id).map_or(0, |k| k.exp.level)))
    }),
    ("GetStockMaidCount", |s, _| Ok(int(s.stock.len() as i64))),
    ("GetSalonGrade", |s, _| Ok(int(s.player.current_salon_grade))),
    ("GetSalonBeautiful", |s, _| Ok(int(s.player.salon_beautiful))),
    ("GetSalonClean", |s, _| Ok(int(s.player.salon_clean))),
    ("GetBGName", |s, _| Ok(Variant::String(s.bg_name.clone()))),
    ("IsItemParam", |s, a| {
        let slot = arg_int(a, 0) as usize;
        let i = s.active_maids.get(slot).copied().flatten().with_context(|| format!("no maid in slot {slot}"))?;
        let tag = arg_string(a, 1);
        let has = s.bodies.get(&i).is_some_and(|b| b.params.iter().any(|p| p.get(&tag).is_some_and(|v| !v.is_empty())));
        Ok(boolean(has))
    }),
    ("IsItemSlot", |s, a| {
        let slot = arg_int(a, 0) as usize;
        let i = s.active_maids.get(slot).copied().flatten().with_context(|| format!("no maid in slot {slot}"))?;
        let mut name = arg_string(a, 1);
        if !crate::chara::SLOTS.iter().any(|(n, _)| *n == name) {
            name = "wear".into();
        }
        Ok(boolean(s.bodies.get(&i).is_some_and(|b| b.loaded_slots.contains(&name))))
    }),
    ("IsMaidClass", |s, a| {
        let c = parse_enum(MAID_CLASS, &arg_string(a, 1)).unwrap_or(0) as usize;
        Ok(boolean(maid(s, a)?.maid_class.get(c).is_some_and(|d| d.is_have)))
    }),
    ("GetMaidClassLevel", |s, a| {
        let c = parse_enum(MAID_CLASS, &arg_string(a, 1)).unwrap_or(0) as usize;
        Ok(int(maid(s, a)?.maid_class.get(c).filter(|d| d.is_have).map_or(0, |d| d.level())))
    }),
    ("IsYotogiClass", |s, a| {
        let c = parse_enum(YOTOGI_CLASS, &arg_string(a, 1)).unwrap_or(0) as usize;
        Ok(boolean(maid(s, a)?.yotogi_class.get(c).is_some_and(|d| d.is_have)))
    }),
    ("GetYotogiClassLevel", |s, a| {
        let c = parse_enum(YOTOGI_CLASS, &arg_string(a, 1)).unwrap_or(0) as usize;
        Ok(int(maid(s, a)?.yotogi_class.get(c).filter(|d| d.is_have).map_or(0, |d| d.level())))
    }),
    ("GetSalonLoan", |s, _| Ok(int(s.player.salon_loan as i32))),
    ("GetSalonLoanPercent", |s, _| {
        let p = &s.player;
        Ok(int((p.salon_loan as f32 / p.init_salon_loan as f32 * 100.).ceil() as i32))
    }),
    ("GetPhase", |s, _| Ok(int(s.player.scenario_phase))),
    ("GetDay", |s, _| Ok(int(s.player.days))),
    ("GetPhaseDay", |s, _| Ok(int(s.player.phase_days))),
    ("CreateStockMaidLoopData", |s, _| {
        let mut list = s.stock.clone();
        s.sort_maid_standard(&mut list);
        s.stock_loop = list;
        Ok(int(s.stock_loop.len() as i64))
    }),
    ("SetActiveSlotFromStockMaidLoopData", |s, a| {
        let i = arg_int(a, 0) as usize;
        let chara = *s.stock_loop.get(i).with_context(|| format!("stock maid loop index {i}"))?;
        set_active(s, 0, chara, false);
        Ok(Variant::Void)
    }),
    ("SetMaidCondition", |s, a| {
        let m = maid(s, a)?;
        match arg_string(a, 1).as_str() {
            "緊張" => m.condition = 1,
            "お近づき" => m.condition = 2,
            "信頼" => m.condition = 3,
            "恋人" => m.condition = 4,
            "愛奴" => m.condition = 5,
            "酔い" => m.condition_special = 1,
            "酔い解除" => m.condition_special = 0,
            _ => {}
        }
        Ok(Variant::Void)
    }),
    ("SetSeikeikenFront", |s, a| {
        let data = s.data()?;
        MaidParam { st: maid(s, a)?, data: &data }.set_seikeiken_front(arg_bool(a, 1));
        Ok(Variant::Void)
    }),
    ("SetSeikeikenBack", |s, a| {
        let data = s.data()?;
        MaidParam { st: maid(s, a)?, data: &data }.set_seikeiken_back(arg_bool(a, 1));
        Ok(Variant::Void)
    }),
    ("SetNewItem", |s, a| {
        let data = s.data()?;
        PlayerParam { st: &mut s.player, data: &data }.add_have_item(&arg_string(a, 0))?;
        Ok(Variant::Void)
    }),
    ("SetPhase", |s, a| {
        let data = s.data()?;
        PlayerParam { st: &mut s.player, data: &data }.set_scenario_phase(arg_int(a, 0) as i32);
        Ok(Variant::Void)
    }),
    ("SetFreeModeLoan", |s, _| {
        let data = s.data()?;
        let mut p = PlayerParam { st: &mut s.player, data: &data };
        p.set_init_salon_loan(INIT_SALON_LOAN_MIN);
        p.set_salon_loan(INIT_SALON_LOAN_MIN);
        Ok(Variant::Void)
    }),
    ("SetContractType", |s, a| {
        let m = maid(s, a)?;
        match arg_string(a, 1).as_str() {
            "育成中" => m.contract_type = 0,
            "フリー" => m.contract_type = 1,
            "専属" => m.contract_type = 2,
            _ => {}
        }
        Ok(Variant::Void)
    }),
    ("SetNewTrophy", |s, a| {
        let data = s.data()?;
        let id = arg_int(a, 0) as i32;
        if (PlayerParam { st: &mut s.player, data: &data }).add_have_trophy(id) {
            s.requests.push(Request::TrophyEffect(id));
        }
        Ok(Variant::Void)
    }),
    ("CheckTrophyContractType", |s, _| {
        let data = s.data()?;
        let contracts: Vec<i32> = s.stock.iter().filter_map(|i| s.maids.get(i)).map(|m| m.contract_type).collect();
        for id in (PlayerParam { st: &mut s.player, data: &data }).check_trophy_contract_type(contracts.into_iter()) {
            s.requests.push(Request::TrophyEffect(id));
        }
        Ok(Variant::Void)
    }),
    ("GetYotogiSelectStageName", |s, _| Ok(Variant::String(s.stage_name.clone()))),
    ("ClearCallStack", |s, _| {
        s.requests.push(Request::ClearCallStack);
        Ok(Variant::Void)
    }),
    ("IsCommunicationChara", |s, _| Ok(boolean(!s.schedule_maids(true, &[0, 2, 3]).is_empty()))),
    ("SetRandomCommuChara", |s, _| {
        let list = s.schedule_maids(true, &[0, 2, 3]);
        anyhow::ensure!(!list.is_empty(), "no communication maid");
        let chara = list[s.random(list.len())];
        set_active(s, 0, chara, true);
        Ok(Variant::Void)
    }),
    ("CreateYotogiCharaList", |s, _| {
        let list = s.schedule_maids(false, &[crate::schedule_data::NIGHT_HAVE_SEX]);
        s.chara_guid_list = list.iter().map(|i| s.maids[i].guid_id.clone()).collect();
        Ok(Variant::Void)
    }),
    ("CreateVipCharaList", |s, _| {
        let list = s.schedule_maids(false, &[crate::schedule_data::NIGHT_VIP, crate::schedule_data::NIGHT_VIP_CALL]);
        s.chara_guid_list = list.iter().map(|i| s.maids[i].guid_id.clone()).collect();
        Ok(Variant::Void)
    }),
    ("IsReceptionChara", |s, _| Ok(boolean(!s.schedule_maids(false, &[crate::schedule_data::NIGHT_ENTERTAIN]).is_empty()))),
    ("SetRandomReceptionChara", |s, _| {
        let list = s.schedule_maids(false, &[crate::schedule_data::NIGHT_ENTERTAIN]);
        anyhow::ensure!(!list.is_empty(), "no entertainment maid");
        let chara = list[s.random(list.len())];
        set_active(s, 0, chara, true);
        Ok(Variant::Void)
    }),
    ("IsSelectChara", |s, _| Ok(boolean(!s.chara_guid_list.is_empty()))),
    ("UpdateMaidRanking", |s, _| {
        update_maid_ranking(s);
        Ok(Variant::Void)
    }),
    ("ResetMaidRanking", |s, _| {
        for i in s.stock.clone() {
            if let Some(m) = s.maids.get_mut(&i) {
                m.popular_rank = 0;
                m.evaluation = 0;
                m.sales = 0;
            }
        }
        s.player.generic_flag.insert("__ランキング日数".into(), 0);
        Ok(Variant::Void)
    }),
    ("SetMaidRanking1st", |s, _| {
        // The C# loop leaves the last stock maid when none is ranked 1st.
        let mut pick = s.stock.last().copied();
        for i in &s.stock {
            if s.maids.get(i).is_some_and(|m| m.popular_rank == 1) {
                pick = Some(*i);
                break;
            }
        }
        let chara = pick.or_else(|| s.stock.first().copied()).context("no stock maid")?;
        set_active(s, 0, chara, true);
        Ok(Variant::Void)
    }),
    ("IsAvailableRanking", |s, _| Ok(boolean(s.player.flag("__ランキング解放") == 1))),
    ("IsMVPEvent", |s, _| {
        if s.player.flag("__ランキング解放") != 1 {
            return Ok(boolean(false));
        }
        if s.player.flag("__ランキング日数") % 7 != 0 {
            return Ok(Variant::Void);
        }
        Ok(boolean(s.stock.iter().any(|i| s.maids.get(i).is_some_and(|m| m.popular_rank == 1))))
    }),
];

fn set_active(s: &mut TjsState, slot: usize, chara: usize, visible: bool) {
    if s.active_maids.len() <= slot {
        s.active_maids.resize(slot + 1, None);
    }
    s.active_maids[slot] = Some(chara);
    s.requests.push(Request::SetActiveMaid { slot, chara, visible });
}

/// `Maid.UpdateMaidRanking`: ranks by evaluation + sales, equal sums share
/// a rank, zero sums stay unranked. (`List.Sort` is unstable in .NET; ties
/// keep stock order here.)
pub fn update_maid_ranking(s: &mut TjsState) {
    if s.player.flag("__ランキング解放") != 1 {
        return;
    }
    let mut list: Vec<(i64, usize)> = Vec::new();
    for i in s.stock.clone() {
        if let Some(m) = s.maids.get_mut(&i) {
            m.popular_rank = 0;
            list.push((m.evaluation + m.sales, i));
        }
    }
    list.sort_by(|a, b| b.0.cmp(&a.0));
    let (mut last, mut rank) = (-1i64, 0);
    for (key, i) in list {
        if key == 0 {
            break;
        }
        if key != last {
            rank += 1;
        }
        if let Some(m) = s.maids.get_mut(&i) {
            m.popular_rank = rank.clamp(0, 99);
        }
        last = key;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptProfile {
    Cm3d2,
    Com3d2,
}

pub(crate) mod retail;
mod standalone;

impl Tjs {
    pub fn new(sources: HashMap<String, String>) -> Result<Self> {
        Self::for_profile(sources, ScriptProfile::Cm3d2)
    }

    pub(crate) fn for_profile(sources: HashMap<String, String>, profile: ScriptProfile) -> Result<Self> {
        Self::with_state(sources,profile,TjsState::default())
    }

    pub(crate) fn with_state(sources: HashMap<String,String>, profile:ScriptProfile,state:TjsState)->Result<Self> {
        let mut runtime = Runtime::with_host(Host { state, sources });
        let scripts = runtime.alloc_ordinary_object();
        runtime.register_object_native(scripts, "execStorage", |rt: &mut Runtime<Host>, _: Option<ObjectHandle>, args: Vec<Variant>| {
            let name = arg_string(&args, 0);
            let source = rt.host_mut().read_text(&name, "")?;
            let file = crysirror_tjs2::compile_source_to_bytecode(&name, &source)?;
            rt.execute_file(&file)
        });
        runtime.set_global_member("Scripts", Variant::Object(scripts));
        let standalone = profile == ScriptProfile::Com3d2;
        if standalone {
            runtime.host_mut().state.player.schedule_slot.resize_with(40, Default::default);
        }
        let overrides = standalone::FUNCS.iter().chain(retail::METHODS);
        let common = FUNCS.iter().filter(|(name, _)| !standalone ||
            (standalone::REGISTERED.contains(name) && !overrides.clone().any(|(override_name, _)| override_name == name)));
        for (name, f) in common.chain(overrides.clone().filter(|_| standalone)) {
            let (name, f) = (*name, *f);
            runtime.register_global_native(name, move |rt: &mut Runtime<Host>, _: Option<ObjectHandle>, a: Vec<Variant>| {
                f(&mut rt.host_mut().state, &a).map_err(|e| crysirror_tjs2::TjsError::runtime(format!("{name}: {e:#}")))
            });
        }
        let mut tjs = Self { runtime };
        if tjs.runtime.host().sources.contains_key("startup.tjs") {
            tjs.exec("startup.tjs", &tjs.runtime.host().sources["startup.tjs"].clone())?;
        }
        Ok(tjs)
    }

    pub fn exec(&mut self, name: &str, source: &str) -> Result<Variant> {
        let file = crysirror_tjs2::compile_source_to_bytecode(name, source).with_context(|| format!("TJS compile {name}"))?;
        Ok(self.runtime.execute_file(&file).with_context(|| format!("TJS execute {name}"))?)
    }

    /// `DLL_TJS_EvalScript`: the value of one expression.
    /// A trailing `;` (`tf['n']++;` in `maid_loop_file.ks`) ends the
    /// expression statement.
    pub fn eval(&mut self, expression: &str) -> Result<Variant> {
        let expression = expression.trim().trim_end_matches(|c: char| c == ';' || c.is_whitespace());
        self.exec("eval", &format!("return ({expression});"))
    }

    pub fn eval_bool(&mut self, expression: &str) -> Result<bool> {
        Ok(match self.eval(expression)? {
            Variant::Void | Variant::Null => false,
            Variant::Integer(n) => n != 0,
            Variant::Real(r) => r != 0.,
            Variant::String(s) => !s.is_empty() && crate::text::atoi(s.as_bytes()) != 0 || (!s.is_empty() && !s.starts_with('0') && s.parse::<f64>().is_err()),
            _ => true,
        })
    }

    pub fn eval_string(&mut self, expression: &str) -> Result<String> {
        Ok(match self.eval(expression)? {
            Variant::Void | Variant::Null => String::new(),
            Variant::Integer(n) => n.to_string(),
            Variant::Real(r) => r.to_string(),
            Variant::String(s) => s,
            other => format!("{other:?}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_script_manager_function_is_registered() {
        let names: Vec<&str> = FUNCS.iter().map(|(n, _)| *n).collect();
        assert_eq!(names.len(), 55);
        let mut tjs = Tjs::new(HashMap::new()).unwrap();
        let s = &mut tjs.runtime.host_mut().state;
        s.active_maids = vec![Some(0)];
        s.maids.insert(0, MaidStatus { seikeiken: 3, ..Default::default() });
        s.maids.get_mut(&0).unwrap().feature.add(18);
        assert_eq!(tjs.eval_string("GetMaidStatus(0, '性経験')").unwrap(), "両方経験あり");
        assert!(tjs.eval_bool("IsMaidFeature(0, '過去の秘密')").unwrap());
        assert!(!tjs.eval_bool("IsMaidFeature(0, '疲労')").unwrap());
        tjs.eval("SetMaidCondition(0, '恋人');").unwrap();
        assert_eq!(tjs.eval_string("GetMaidStatus(0, '状態')").unwrap(), "恋人");
    }
}

/// The plug-in's `Dictionary.toStructString` (cm3d2_x64.dll `0x180081380`,
/// members by `DictMemberDispCaller` `0x1800815d0`, arrays `0x18006c870`):
/// `(const)%[key,value,…]` and `(const)[value,…]` without spaces; strings
/// quoted with only `"` and `\` escaped (`""` when empty), integers in
/// decimal, `void`, octets as `<% xx %>`. Member order is the dictionary's
/// own; loading evaluates the literal, so order carries no meaning.
/// Reals use Rust's shortest representation; the native real formatter
/// (`0x1800215f0`) is not traced (no `KAGParser.store()` member is real).
pub fn to_struct_string(rt: &crysirror_tjs2::runtime::Runtime<Host>, v: &Variant) -> String {
    fn scalar(v: &Variant) -> String {
        match v {
            Variant::Void => "void".into(),
            Variant::Null => "null".into(),
            Variant::Integer(n) => n.to_string(),
            Variant::Real(r) => format!("{r:?}"),
            Variant::String(s) if s.is_empty() => "\"\"".into(),
            Variant::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
            Variant::Octet(b) => format!("<%{}%>", b.iter().map(|x| format!("{x:02x} ")).collect::<String>()),
            _ => "void".into(),
        }
    }
    fn value(rt: &crysirror_tjs2::runtime::Runtime<Host>, v: &Variant, out: &mut String) {
        let Variant::Object(h) = v else {
            out.push_str(&scalar(v));
            return;
        };
        if let Some(items) = rt.array_elements(*h) {
            out.push_str("(const)[");
            for (i, e) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                value(rt, e, out);
            }
            out.push(']');
        } else {
            out.push_str("(const)%[");
            for (i, (k, e)) in rt.object_members(*h).iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&scalar(&Variant::String(k.clone())));
                out.push(',');
                value(rt, e, out);
            }
            out.push(']');
        }
    }
    let mut out = String::new();
    value(rt, v, &mut out);
    out
}

#[cfg(test)]
mod struct_string_tests {
    use super::*;

    #[test]
    fn struct_strings_evaluate_back() {
        let mut tjs = Tjs::new(HashMap::new()).unwrap();
        let v = tjs.eval(r#"%["storageName" => "a\\b.ks", "curLine" => 12, "callStack" => [%["label" => "*x", "pos" => 0]], "macros" => %[]]"#).unwrap();
        let text = to_struct_string(&tjs.runtime, &v);
        assert!(text.starts_with("(const)%[") && !text.contains(' '), "{text}");
        let back = tjs.eval(&text).unwrap();
        let h = back.object_handle().unwrap();
        assert_eq!(tjs.runtime.object_member(h, "storageName").to_tjs_string().unwrap(), "a\\b.ks");
        assert_eq!(tjs.runtime.object_member(h, "curLine").to_integer().unwrap(), 12);
        let frames = tjs.runtime.object_member(h, "callStack").object_handle().unwrap();
        let f = tjs.runtime.array_elements(frames).unwrap()[0].object_handle().unwrap();
        assert_eq!(tjs.runtime.object_member(f, "label").to_tjs_string().unwrap(), "*x");
    }
}
