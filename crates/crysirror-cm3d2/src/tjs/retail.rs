// SPDX-License-Identifier: MPL-2.0
//! Standalone ScriptManager services. NEI identifiers and authored thresholds
//! belong to the mounted game and may differ between installations.
use super::{Func, TjsState, Variant, arg_bool, arg_int, arg_string, boolean, int, maid};
use crate::{files::GameFiles, native_key::NativeKeys, nei::Table};
mod activities;
mod character;
mod facility;
mod day;
mod stock;
pub(crate) use stock::{retail_maid,prop,edit_wear,edit_slider,apply_body as apply_live_body,select as select_stock};
mod stock_profile;
pub(crate) use day::start_before_hiring;
mod preset;
pub(crate) use preset::apply_bytes as apply_preset_bytes;
pub(crate) use character::MPN as RETAIL_MPN;
mod scenario;
mod selection;
mod status;
use anyhow::{Context, Result, bail, ensure};
use chrono::{Datelike, Local};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::ClientStorage,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone)]
struct Source {
    files: Arc<GameFiles>,
    storage: Arc<dyn ClientStorage>,
    dll: VfsPath,
    dir: String,
}

#[derive(Clone, Default)]
pub(crate) struct State {
    source: Option<Source>,
    key: Option<NativeKeys>,
    tables: BTreeMap<String, Arc<Table>>,
    legacy_files: GameFiles,
    enabled_lists: BTreeMap<String, BTreeSet<i32>>,
    suffixes: Option<Vec<String>>,
    event_end: BTreeMap<String, BTreeMap<i32, bool>>,
    pub(crate) compatibility: bool,
    pub(crate) adv_depth: (usize, usize),
    switch_depth: (usize, usize),
    pub(crate) stop_motions: bool,
    activities: activities::Activities,
    events: scenario::Events,
    vr_karaoke: Option<bool>,
    selection: Option<selection::Manager>,
    materials: BTreeSet<i32>,
    pub(crate) peak_facilities: usize,
    facility_levels: BTreeMap<i32, i32>,
    facilities: Vec<facility::Facility>,
    pub(crate) stock_models: BTreeMap<usize, Arc<crate::chara::Maid>>,
    stock_names: Option<Arc<[Vec<String>;2]>>,
    appearances: BTreeMap<usize, preset::Appearance>,
    pub(crate) edit_owned_only: bool,
    owned_parts: BTreeMap<String, bool>,
    parts_indexed: bool,
    characters: BTreeMap<String, character::Status>,
    main_characters: BTreeSet<usize>,
    debug_stage: String,
    pub(crate) adv_present: bool,
    pub(crate) title_request: Option<String>,
    kasizuki: Kasizuki,
    buildings: Option<BTreeSet<i32>>,
}

#[derive(Clone)]
struct Kasizuki {
    system: BTreeMap<String, String>,
    maids: BTreeMap<String, BTreeMap<String, i32>>,
}
impl Default for Kasizuki {
    fn default() -> Self {
        Self {
            system: BTreeMap::from([("男種類".into(), "10".into())]),
            maids: Default::default(),
        }
    }
}

impl State {
    pub(crate) fn apply_model_colors(&self, id:usize, model:&mut crate::chara::Maid) {
        if let Some(colors)=self.appearances.get(&id).and_then(|a|a.colors) {
            for (i,c) in colors.into_iter().enumerate(){model.set_parts_color(i,c);}
        }
    }

    pub(crate) fn attach(
        &mut self,
        files: Arc<GameFiles>,
        storage: Arc<dyn ClientStorage>,
        dir: &str,
    ) -> Result<()> {
        self.source = Some(Source {
            files,
            storage,
            dir: dir.into(),
            dll: VfsPath::new(Mount::Asset, &format!("{dir}/Plugins/cm3d2.dll"))?,
        });
        self.key = None;
        self.stock_names = None;
        self.tables.clear();
        self.enabled_lists.clear();
        self.suffixes = None;
        Ok(())
    }

    pub(crate) fn audio_files<'a>(
        &'a self,
        normal: &'a GameFiles,
        kind: crate::sound::Kind,
        file: &str,
        legacy: bool,
    ) -> Option<&'a GameFiles> {
        let (first, second) = if legacy {
            (&self.legacy_files, normal)
        } else {
            (normal, &self.legacy_files)
        };
        if first.exists(file) {
            Some(first)
        } else if kind == crate::sound::Kind::Voice && second.exists(file) {
            Some(second)
        } else {
            None
        }
    }
    pub(crate) fn files<'a>(&'a self, normal: &'a GameFiles) -> &'a GameFiles {
        if self.compatibility {
            &self.legacy_files
        } else {
            normal
        }
    }
    pub(crate) fn returned_to(&mut self, depth: (usize, usize)) {
        if self.compatibility && depth == self.switch_depth {
            self.compatibility = false;
            self.stop_motions = true;
        }
    }
    fn source(&self) -> Result<&Source> {
        self.source
            .as_ref()
            .context("standalone game archives are not attached")
    }

    pub(crate) fn enabled(&mut self, base: &str, id: i32) -> Result<bool> {
        if let Some(ids) = self.enabled_lists.get(base) {
            return Ok(ids.contains(&id));
        }
        let source = self.source()?.clone();
        if self.suffixes.is_none() {
            let path = VfsPath::new(Mount::Asset, "GameData/paths.dat")?;
            let bytes = source.storage.read(&path)?;
            let mut r = crate::binio::Reader::new(&bytes);
            r.expect_str("CM3D2_PATHS")?;
            let _version = r.i32()?;
            let n = r.count()?;
            ensure!(n <= 4096, "too many archive suffixes");
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                let suffix = r.str()?;
                ensure!(
                    !suffix.contains(['/', '\\', '\0']),
                    "invalid archive suffix"
                );
                v.push(suffix);
            }
            self.suffixes = Some(v);
        }
        let mut names = vec![format!("{base}.nei")];
        names.extend(
            self.suffixes
                .iter()
                .flatten()
                .map(|suffix| format!("{base}_{suffix}.nei")),
        );
        let mut ids = BTreeSet::new();
        for name in names {
            if source.files.exists(&name) {
                let t = self.table(&name)?;
                ids.extend((1..t.rows).filter(|&r| t.exists(0, r)).map(|r| t.int(0, r)));
                // Keep the compact ID union. Extension tables are only needed during this pass.
                self.tables.remove(&name);
            }
        }
        let result = ids.contains(&id);
        self.enabled_lists.insert(base.into(), ids);
        Ok(result)
    }

    /// PlayerStatus.Status.CreateHaveItemList reads enabled shop parts, valid
    /// set cards and the scenario award table. Items outside this set are owned.
    pub(crate) fn editor_files(&self)->Result<(Arc<GameFiles>,Arc<dyn ClientStorage>)> {
        let source=self.source()?;
        Ok((source.files.clone(),source.storage.clone()))
    }
    pub(crate) fn editor_owns(&self, menu:&str)->bool { self.owned_parts.get(&menu.to_lowercase()).copied().unwrap_or(true) }
    pub(crate) fn index_parts(&mut self) -> Result<()> {
        if self.parts_indexed { return Ok(()); }
        let mut items = BTreeSet::new();
        let mut locked = BTreeSet::new();
        for (name, parts) in [("game_in_shop_item_setting.nei", true),
                              ("game_in_shop_item_groups_setting.nei", false),
                              ("game_in_shop_event_item_setting.nei", false)] {
            if name == "game_in_shop_event_item_setting.nei" && !self.source()?.files.exists(name) { continue; }
            let table = self.table(name)?;
            for row in 1..table.rows {
                let id = table.int(0,row);
                if !table.exists(0,row) || !self.enabled("game_in_shop_itemlist",id)? { continue; }
                ensure!(items.insert(id),"duplicate shop id {id}");
                if parts {
                    for col in (9..table.columns).filter(|&c|table.exists(c,row)) {
                        locked.insert(format!("{}.menu",table.string(col,row).to_lowercase()));
                    }
                }
            }
        }
        let cards=self.table("game_in_shop_setcard_setting.nei")?;
        for row in (1..cards.rows).filter(|&r|cards.exists(0,r) && cards.exists(1,r)) {
            let mut available=true;
            for col in (1..cards.columns).take_while(|&c| cards.exists(c,row)) {
                let id=cards.string(col,row).trim().parse::<i32>()?;
                if !items.contains(&id) { available=false; break; }
            }
            if available {
                for item in cards.string(0,row).split(',') { locked.insert(format!("{}.menu",item.to_lowercase())); }
            }
        }
        let awards=self.table("scenario_get_item_list.nei")?;
        for row in (0..awards.rows).filter(|&r|awards.exists(0,r)) {
            locked.insert(format!("{}.menu",awards.string(0,row).to_lowercase()));
        }
        for name in locked { self.owned_parts.entry(name).or_insert(false); }
        self.parts_indexed=true;
        Ok(())
    }

    pub(crate) fn table(&mut self, name: &str) -> Result<Arc<Table>> {
        if let Some(table) = self.tables.get(name) {
            return Ok(table.clone());
        }
        let source = self.source()?.clone();
        if self.key.is_none() {
            self.key = Some(NativeKeys::from_dll(&source.storage.read(&source.dll)?)?);
        }
        let bytes = source.files.read(source.storage.as_ref(), name)?;
        let table = Arc::new(
            Table::open(&bytes, &self.key.as_ref().unwrap().table_key)
                .with_context(|| format!("standalone table {name}"))?,
        );
        self.tables.insert(name.to_owned(), table.clone());
        Ok(table)
    }
}

fn row_id(table: &Table, id: i32) -> Result<usize> {
    (1..table.rows)
        .find(|&r| table.exists(0, r) && table.int(0, r) == id)
        .with_context(|| format!("unknown table ID {id}"))
}
fn arity(args: &[Variant], count: usize) -> Result<()> {
    ensure!(
        args.len() == count,
        "expected {count} arguments, got {}",
        args.len()
    );
    Ok(())
}
fn enum_value(names: &[&str], text: &str) -> Option<i32> {
    let text = text.trim();
    if let Ok(i) = text.parse() {
        return Some(i);
    }
    text.split(',').try_fold(0, |value, name| {
        Some(value | names.iter().position(|item| *item == name.trim())? as i32)
    })
}
fn number(s: &State, name: &str) -> i32 {
    s.kasizuki
        .system
        .get(name)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}
fn maid_flag(s: &TjsState, slot: usize, name: &str) -> Result<i32> {
    let maid = s
        .maid(slot)
        .with_context(|| format!("no maid in slot {slot}"))?;
    if name == "好感度＿本編" {
        return Ok(maid.likability);
    }
    Ok(s.retail
        .kasizuki
        .maids
        .get(&maid.guid_id)
        .and_then(|m| m.get(name))
        .copied()
        .unwrap_or(0))
}
fn likability(s: &TjsState, slot: usize) -> Result<i32> {
    match number(&s.retail, "男種類") {
        10 => maid_flag(s, slot, "好感度＿本編"),
        20 => maid_flag(s, slot, "好感度＿傅き男１"),
        _ => Ok(-1),
    }
}
fn threshold_name(table: &Table, value: i32) -> Result<String> {
    let row = (1..table.rows)
        .filter(|&r| table.int(0, r) <= value)
        .max_by_key(|&r| table.int(0, r))
        .context("no matching threshold")?;
    let name = table.string(1, row);
    ensure!(!name.is_empty(), "empty threshold name");
    Ok(name)
}

fn building(s: &mut TjsState, a: &[Variant], upward: bool) -> Result<Variant> {
    arity(a, 1)?;
    let t = s.retail.table("facility.nei")?;
    let name = arg_string(a, 0);
    let r = (1..t.rows)
        .find(|&r| t.exists(0, r) && t.string(2, r) == name)
        .with_context(|| format!("unknown facility {name}"))?;
    let id = if upward {
        if !t.exists(5, r) {
            return Ok(boolean(false));
        }
        t.int(5, r)
    } else {
        t.int(0, r)
    };
    Ok(boolean(facility::built(s, &t, id)))
}

fn plugin(s: &mut TjsState, a: &[Variant]) -> Result<Variant> {
    arity(a, 1)?;
    if s.retail.compatibility {
        return Ok(boolean(false));
    }
    let name = arg_string(a, 0).to_lowercase();
    let t = s.retail.table("plugin_check_list.nei")?;
    let Some(r) = (1..t.rows).find(|&r| t.exists(0, r) && t.string(1, r).to_lowercase() == name)
    else {
        return Ok(boolean(false));
    };
    let files = &s.retail.source()?.files;
    let legacy = t.string(2, r) == "〇";
    // GameUty passes null to PluginData when there is no linked legacy installation.
    if legacy && s.retail.legacy_files.is_empty() {
        return Ok(boolean(false));
    }
    let matches = (3..t.columns)
        .take_while(|&col| t.exists(col, r))
        .all(|col| {
            let file = t.string(col, r).trim().to_lowercase();
            if legacy {
                s.retail.legacy_files.exists(&file)
                    || (file
                        .rsplit_once('.')
                        .is_some_and(|(_, ext)| matches!(ext, "menu" | "mate" | "tex"))
                        && files.exists(&file))
            } else {
                files.exists(&file)
            }
        });
    Ok(boolean(matches))
}

fn maid_trait(s: &mut TjsState, a: &[Variant], feature: bool, write: bool) -> Result<Variant> {
    arity(a, if write { 3 } else { 2 })?;
    let prefix = if feature {
        "maid_status_feature"
    } else {
        "maid_status_propensity"
    };
    let t = s.retail.table(&format!("{prefix}_list.nei"))?;
    let name = arg_string(a, 1);
    let row = (1..t.rows)
        .find(|&r| t.exists(0, r) && t.string(1, r) == name)
        .with_context(|| format!("unknown {prefix} {name}"))?;
    let id = t.int(0, row);
    let enabled = write && s.retail.enabled(&format!("{prefix}_enabled_list"), id)?;
    let m = maid(s, a)?;
    if !write {
        return Ok(boolean(if feature {
            m.is_feature(id)
        } else {
            m.is_propensity(id)
        }));
    }
    // Native code converts the third argument to bool, then calls Add even when false.
    if enabled {
        if feature {
            m.feature.add(id);
        } else {
            m.propensity.add(id);
        }
    }
    Ok(Variant::Void)
}

fn award(s:&mut TjsState,id:i32)->Result<()> {
    if !s.retail.enabled("trophy_enabled_list",id)? {return Ok(());}
    let t=s.retail.table("trophy_list.nei")?;
    let Ok(r)=row_id(&t,id) else {return Ok(());};
    // In 1.0.4 the Data constructor never assigns lineupIdArray.
    if s.player.have_trophy.add(id) && matches!(t.string(4,r).as_str(),"ヒロイン"|"システム") {
        s.requests.push(crate::tjs::Request::TrophyEffect(id));
    }
    Ok(())
}
fn contract_awards(s:&mut TjsState,_:&[Variant])->Result<Variant>{
    if s.retail.compatibility || s.player.flag("2.0モード")==1 {return Ok(Variant::Void);}
    let mut counts=[0usize;3];
    for id in &s.stock {
        let m=s.maids.get(id).context("missing stock status")?;
        let x=s.retail.characters.get(&m.guid_id);
        if x.is_some_and(|x|x.sub_name.is_some()){continue;}
        counts[0]+=usize::from(m.contract_type==1);
        counts[1]+=usize::from(m.contract_type==2);
        counts[2]+=usize::from(x.is_some_and(|x|x.relation==2));
    }
    for (count,awards) in counts.into_iter().zip([&[(1,10),(3,20),(5,30)][..],&[(1,40),(3,50),(5,60)],&[(1,70),(3,80)]]) {
        for &(min,id) in awards {if count>=min {award(s,id)?;}}
    }
    Ok(Variant::Void)
}
fn facility_characters(s: &mut TjsState, _: &[Variant]) -> Result<Variant> {
    let t = s.retail.table("schedule_define.nei")?;
    let work = (0..t.rows)
        .filter(|&r| t.string(0, r) == "faclilityPowerUpWorkId")
        .map(|r| t.int(1, r))
        .next_back()
        .unwrap_or(3000);
    let day = s.player.flag("時間帯") != 3;
    s.chara_guid_list = s
        .player
        .schedule_slot
        .iter()
        .take(40)
        .filter(|slot| !slot.maid_guid.is_empty())
        .filter_map(|slot| {
            let m = s
                .stock
                .iter()
                .filter_map(|i| s.maids.get(i))
                .find(|m| m.guid_id == slot.maid_guid)?;
            ((if day { m.noon_work_id } else { m.night_work_id }) == work)
                .then(|| m.guid_id.clone())
        })
        .collect();
    Ok(Variant::Void)
}

fn event_end(s: &mut TjsState, a: &[Variant]) -> Result<Variant> {
    arity(a, 3)?;
    let slot = arg_int(a, 0) as usize;
    let Some(index) = s
        .active_maids
        .get(slot)
        .copied()
        .flatten()
        .or_else(|| s.stock.get(slot).copied())
    else {
        return Ok(Variant::Void);
    };
    let guid = s
        .maids
        .get(&index)
        .context("missing character status")?
        .guid_id
        .clone();
    let id = arg_int(a, 1) as i32;
    if s.retail.enabled("selectable_scenario_id", id)? {
        let t = s.retail.table("select_scenario_data.nei")?;
        if row_id(&t, id).is_ok() {
            s.retail
                .event_end
                .entry(guid)
                .or_default()
                .insert(id, arg_bool(a, 2));
        }
    }
    Ok(Variant::Void)
}

pub(super) const METHODS: &[(&str, Func)] = &[
    ("CreateNewMaid", stock::create),
    ("SetTmpFlag", |s, a| {
        arity(a, 2)?;
        let value = arg_int(a, 1) as i32;
        s.tmp_flags.insert(arg_string(a, 0), value.into());
        Ok(int(value))
    }),
    ("GetTmpFlag", |s, a| {
        arity(a, 1)?;
        Ok(int(s.tmp_flags.get(&arg_string(a, 0)).copied().unwrap_or(0)))
    }),
    ("GetYotogiSkillLevel", |s, a| {
        arity(a, 2)?;
        let id = arg_int(a, 1) as i32;
        Ok(int(maid(s, a)?.skill_data.get(&id).map_or(0, |k| k.exp.level)))
    }),
    ("IsSeikantai", character::legacy_sensitivity),
    ("SetSeikeikenFront", |s, a| character::set_experience(s, a, 1)),
    ("SetSeikeikenBack", |s, a| character::set_experience(s, a, 2)),
    ("SetNewTrophy",|s,a|{
        if !s.retail.compatibility && s.player.flag("2.0モード")!=1 {arity(a,1)?;award(s,arg_int(a,0) as i32)?;}
        Ok(Variant::Void)
    }),
    ("CheckTrophyContractType",contract_awards),
    ("GetMaidStatus", status::read),
    ("GetMaidFlag", |s, a| status::flag(s, a, false)),
    ("SetMaidFlag", |s, a| status::flag(s, a, true)),
    ("SetContractType", |s, a| {
        arity(a, 2)?;
        let name = arg_string(a, 1);
        let value = ["育成中", "フリー", "専属"]
            .iter()
            .position(|n| *n == name)
            .context("unknown contract type")?;
        maid(s, a)?.contract_type = value as i32;
        Ok(Variant::Void)
    }),
    ("IsVRKaraokeMode", |s, _| {
        Ok(int(i32::from(
            s.retail
                .vr_karaoke
                .context("SceneVRCommunication is not active")?,
        )))
    }),
    ("IsYotogiSubCharaSelect", selection::sub_select),
    ("IsSelectedYotogiSkill", selection::selected),
    ("GetYotogiSkillSpecialType", selection::special),
    ("GetEventAdded", scenario::added),
    ("IsCommunicationChara", facility::has_communication),
    ("SetActiveSlotFromStockMaidLoopData", |s,a| {
        arity(a,1)?;
        let index = arg_int(a,0) as usize;
        let id = *s.stock_loop.get(index).context("invalid stock loop index")?;
        s.retail.stop_motions = true;
        facility::activate(s,0,id,false);
        Ok(Variant::Void)
    }),
    ("SetMaidRanking1st", |s,_| {
        let id = s.stock.iter().copied().find(|i| s.maids.get(i).is_some_and(|m| m.popular_rank == 1))
            .or_else(||s.stock.last().copied().filter(|i|s.maids.contains_key(i)))
            .or_else(||s.stock.first().copied()).context("no stock maid")?;
        s.retail.stop_motions = true;
        facility::activate(s,0,id,true);
        Ok(Variant::Void)
    }),
    ("SetRandomCommuChara", facility::random_communication),
    ("CreateYotogiCharaList", |s, _| facility::select_work(s, &["HaveSex"])),
    ("CreateVipCharaList", |s, _| facility::select_work(s, &["Vip", "VipCall"])),
    ("SetRandomReceptionChara", |s, _| facility::reception(s, true)),
    ("IsReceptionChara", |s, _| facility::reception(s, false)),
    ("SetCommunicationChara", facility::communication),
    ("SetMaidCostumeFacility", |s, a| {
        facility::costume(s, a, false)
    }),
    ("ResetMaidCostumeFacility", |s, a| {
        facility::costume(s, a, true)
    }),
    ("IsItemSlot", |s,a| {
        arity(a,2)?;
        let slot=arg_int(a,0) as usize;
        let id=s.active_maids.get(slot).copied().flatten().with_context(||format!("no maid in slot {slot}"))?;
        let tag=arg_string(a,1);
        let tag=tag.trim();
        let name=if let Ok(index)=tag.parse::<usize>() {
            crate::chara::SLOTS.get(index)
        } else {
            crate::chara::SLOTS.iter().find(|(name,_)|*name==tag)
        }.with_context(||format!("unknown TBody.SlotID {tag}"))?.0;
        Ok(boolean(s.bodies.get(&id).is_some_and(|b|b.loaded_slots.contains(name))))
    }),
    ("SetNewItem", |s,a| {
        arity(a,1)?;
        if !s.retail.compatibility && s.player.flag("2.0モード") != 1 {
            let name=arg_string(a,0).to_lowercase();
            s.retail.index_parts()?;
            *s.retail.owned_parts.get_mut(&name).with_context(||format!("unregistered item {name}"))? = true;
        }
        Ok(Variant::Void)
    }),
    ("PresetLoad", preset::load),
    ("SetDanceData", activities::set_dance),
    ("CheckDanceType", activities::check_dance),
    ("GetChallengeRank", activities::challenge_rank),
    ("CasinoShopBuy", activities::bought),
    (
        "CreateCompetitiveShowLoopData",
        activities::create_show_loop,
    ),
    ("GetCompetitiveShowLoopCount", activities::show_count),
    ("CallTitleBar", |s, a| {
        arity(a, 1)?;
        if s.retail.adv_present {
            s.retail.title_request = Some(arg_string(a, 0));
        }
        Ok(Variant::Void)
    }),
    ("SetCompatibilityMode", |s, a| {
        arity(a, 1)?;
        s.retail.compatibility = arg_bool(a, 0);
        s.retail.switch_depth = s.retail.adv_depth;
        s.retail.stop_motions = true;
        Ok(Variant::Void)
    }),
    ("GetRentalNPCName", |s, _| {
        ensure!(
            s.retail.compatibility || s.player.flag("2.0モード") == 1,
            "requires legacy mode"
        );
        Ok(Variant::String(String::new()))
    }),
    ("IsMarriage", |s, a| character::old_flag(s, a, true)),
    ("IsNewWifeFlag", |s, a| character::old_flag(s, a, false)),
    ("SetMaidCondition", character::set_condition),
    ("IsKasizukiCorrectSenarioFlag", character::eligible),
    ("AddMaidStatus", character::add_status),
    ("IsMaidFeature", |s, a| maid_trait(s, a, true, false)),
    ("SetFeature", |s, a| maid_trait(s, a, true, true)),
    ("IsMaidPropensity", |s, a| maid_trait(s, a, false, false)),
    ("SetPropensity", |s, a| maid_trait(s, a, false, true)),
    ("CreateFacilityCharaList", facility_characters),
    ("SetEventEndFlag", event_end),
    ("GetMenuFileNameFromMaidMPN", |s, a| {
        character::prop(s, a, true)
    }),
    ("GetMaidMPN", |s, a| character::prop(s, a, false)),
    ("IsJobMaid", |s, a| {
        character::class(s, a, true, false, false)
    }),
    ("GetJobMaidLevel", |s, a| {
        character::class(s, a, true, true, false)
    }),
    ("IsMaidClass", |s, a| {
        character::class(s, a, true, false, true)
    }),
    ("GetMaidClassLevel", |s, a| {
        character::class(s, a, true, true, true)
    }),
    ("IsYotogiClass", |s, a| {
        character::class(s, a, false, false, false)
    }),
    ("GetYotogiClassLevel", |s, a| {
        character::class(s, a, false, true, false)
    }),
    ("SetMaidLeader", |s, a| {
        arity(a, 2)?;
        if let Some(maid) = s.maid_mut(arg_int(a, 0) as usize) {
            maid.leader = arg_bool(a, 1);
        }
        Ok(Variant::Void)
    }),
    ("SetMainCharaFlag", |s, a| {
        arity(a, 2)?;
        if let Some(index) = s
            .active_maids
            .get(arg_int(a, 0) as usize)
            .copied()
            .flatten()
        {
            if arg_bool(a, 1) {
                s.retail.main_characters.insert(index);
            } else {
                s.retail.main_characters.remove(&index);
            }
        }
        Ok(Variant::Void)
    }),
    ("IsYotogiSkill", |s, a| {
        arity(a, 2)?;
        Ok(boolean(
            maid(s, a)?
                .skill_data
                .get(&(arg_int(a, 1) as i32))
                .is_some(),
        ))
    }),
    ("GetNoonScuccessLevel", |s, a| {
        let [Variant::Integer(slot)] = a else {
            bail!("expected integer maid slot");
        };
        let guid = &s.maid(*slot as usize).context("empty maid slot")?.guid_id;
        Ok(int(s
            .player
            .schedule_slot
            .iter()
            .find(|v| &v.maid_guid == guid)
            .map_or(-1, |v| v.noon_success_level)))
    }),
    ("IsOldPersonal", |s, a| {
        arity(a, 1)?;
        let Some(personal) = s.maid(arg_int(a, 0) as usize).map(|m| m.personal) else {
            return Ok(boolean(false));
        };
        let t = s.retail.table("maid_status_personal_list.nei")?;
        Ok(boolean(t.string(5, row_id(&t, personal)?) == "〇"))
    }),
    ("SetInitSeikeiken", |s, a| {
        arity(a, 2)?;
        if let Some(v) = enum_value(&["No_No", "Yes_No", "No_Yes", "Yes_Yes"], &arg_string(a, 1)) {
            maid(s, a)?.init_seikeiken = v;
        }
        Ok(Variant::Void)
    }),
    // Literal bodies in the standalone assembly: unconditional assertion and empty method respectively.
    ("PresetLoadNPC", |_, _| {
        bail!("standalone ScriptManager explicitly rejects PresetLoadNPC")
    }),
    ("SetSexualParam", |_, _| Ok(Variant::Void)),
    ("SetYotogiSelectStageName", |s, a| {
        arity(a, 1)?;
        s.retail.debug_stage = arg_string(a, 0);
        Ok(Variant::Void)
    }),
    ("GetDateYear", |_, _| Ok(int(Local::now().year()))),
    ("GetDateMonth", |_, _| Ok(int(Local::now().month()))),
    ("GetDateDay", |_, _| Ok(int(Local::now().day()))),
    ("IsExistentFile", |s, a| {
        if s.retail.compatibility {
            return Ok(boolean(false));
        }
        arity(a, 1)?;
        Ok(boolean(s.retail.source()?.files.exists(&arg_string(a, 0))))
    }),
    ("IsExistentFileLegacy", |s, a| {
        arity(a, 1)?;
        Ok(boolean(s.retail.legacy_files.exists(&arg_string(a, 0))))
    }),
    ("IsPluginPack", plugin),
    ("IsBuildingFacility", |s, a| building(s, a, false)),
    ("IsBuildingFacilityUpward", |s, a| building(s, a, true)),
    ("SetMusicRelease", |s, a| {
        arity(a, 1)?;
        let level = arg_int(a, 0) as i32;
        if s.player.flag("_楽曲解放") <= level {
            s.player.generic_flag.insert("_楽曲解放".into(), level);
        }
        Ok(Variant::Void)
    }),
    ("BjDayFirst", |s, a| {
        let day = s.player.days;
        let first = s.player.flag("BJ最終プレイ日") != day;
        if (0..a.len()).any(|i| arg_string(a, i) == "ブラックジャック終了") {
            s.player.generic_flag.insert("BJ最終プレイ日".into(), day);
        }
        Ok(boolean(first))
    }),
    ("IsKasizukiGuest", |s, _| {
        Ok(boolean(number(&s.retail, "男種類") != 10))
    }),
    ("GetKasizukiSystemFlag", |s, a| {
        arity(a, 1)?;
        Ok(Variant::String(
            s.retail
                .kasizuki
                .system
                .get(&arg_string(a, 0))
                .cloned()
                .unwrap_or_default(),
        ))
    }),
    ("SetKasizukiSystemFlag", |s, a| {
        arity(a, 2)?;
        s.retail
            .kasizuki
            .system
            .insert(arg_string(a, 0), arg_string(a, 1));
        Ok(Variant::Void)
    }),
    ("AddKasizukiSystemFlag", |s, a| {
        arity(a, 2)?;
        let name = arg_string(a, 0);
        let next = number(&s.retail, &name).wrapping_add(arg_int(a, 1) as i32);
        s.retail.kasizuki.system.insert(name, next.to_string());
        Ok(Variant::Void)
    }),
    ("SetKasizukiMaidFlag", |s, a| {
        arity(a, 3)?;
        let guid = maid(s, a)?.guid_id.clone();
        let data = s.retail.kasizuki.maids.entry(guid).or_default();
        let name = arg_string(a, 1);
        if name != "好感度＿本編" {
            data.insert(name, arg_int(a, 2) as i32);
        }
        Ok(Variant::Void)
    }),
    ("GetKasizukiMaidFlag", |s, a| {
        arity(a, 2)?;
        Ok(int(maid_flag(
            s,
            arg_int(a, 0) as usize,
            &arg_string(a, 1),
        )?))
    }),
    ("GetKasizukiMaidLikability", |s, _| {
        Ok(int(likability(s, 0)?))
    }),
    ("GetKasizukiMaidWorkCount", |s, _| {
        Ok(int(maid_flag(s, 0, "仕事回数")?))
    }),
    ("GetKasizukiMaidLikabilityName", |s, _| {
        let v = likability(s, 0)?;
        let t = s.retail.table("kasizuki_likability_convert_list.nei")?;
        Ok(Variant::String(threshold_name(&t, v)?))
    }),
    ("GetKasizukiMaidWorkCountName", |s, _| {
        let v = maid_flag(s, 0, "仕事回数")?;
        let t = s.retail.table("kasizuki_work_count_convert_list.nei")?;
        Ok(Variant::String(threshold_name(&t, v)?))
    }),
    ("GetKasizukiSelectRoomName", |s, _| {
        let id = number(&s.retail, "選択部屋");
        let t = s.retail.table("kasizuki_room_list.nei")?;
        Ok(Variant::String(t.string(1, row_id(&t, id)?)))
    }),
    ("SetKasizukiPassFlag", |s, a| {
        arity(a, 1)?;
        let id = arg_int(a, 0) as i32;
        let t = s.retail.table("kasizuki_play_list.nei")?;
        row_id(&t, id)?;
        s.retail
            .kasizuki
            .system
            .insert(format!("プレイ実行＿{id}"), "True".into());
        Ok(Variant::Void)
    }),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tjs::{ScriptProfile, Tjs};

    pub(super) fn script() -> Result<Tjs> {
        let mut t = Tjs::for_profile(Default::default(), ScriptProfile::Com3d2)?;
        let s = &mut t.runtime.host_mut().state;
        s.active_maids = vec![Some(0), Some(1)];
        s.stock = vec![0, 1];
        s.next_character = 2;
        for i in 0..2 {
            let m = s.maids.entry(i).or_default();
            m.guid_id = format!("character-{i}");
            m.likability = 10 + i as i32;
        }
        Ok(t)
    }

    #[test]
    fn retail_slot_uses_the_requested_native_body_slot() -> Result<()> {
        let mut vm=script()?;
        let id=vm.runtime.host().state.active_maids[0].unwrap();
        let body=vm.runtime.host_mut().state.bodies.entry(id).or_default();
        body.loaded_slots.insert("wear".to_owned());
        assert!(vm.eval_bool("IsItemSlot(0,'wear') && IsItemSlot(0,'7') && !IsItemSlot(0,'head')")?);
        assert!(vm.eval("IsItemSlot(0,'Wear')").is_err());
        assert!(vm.eval("IsItemSlot(0,'end')").is_err());
        Ok(())
    }

    #[test]
    fn retail_registration_is_callable_in_tjs() -> Result<()> {
        let mut t = script()?;
        let names = super::super::standalone::REGISTERED;
        assert_eq!(names.len(), 128);
        assert_eq!(names.iter().collect::<BTreeSet<_>>().len(), 128);
        for name in names {
            assert!(
                !matches!(t.eval(name)?, Variant::Void | Variant::Null),
                "missing {name}"
            );
        }
        Ok(())
    }

    #[test]
    fn standalone_flags_follow_character_and_day() -> Result<()> {
        let mut t = script()?;
        assert_eq!(t.runtime.host().state.player.schedule_slot.len(), 40);
        t.exec("flags","SetKasizukiMaidFlag(0,'仕事回数',7);SetKasizukiMaidFlag(1,'仕事回数',2);SetKasizukiMaidFlag(0,'好感度＿本編',999);")?;
        assert!(t.eval_bool(
            "GetKasizukiMaidLikability()==10 && GetKasizukiMaidWorkCount()==7 && !IsKasizukiGuest()"
        )?);
        t.runtime.host_mut().state.active_maids.swap(0, 1);
        assert!(t.eval_bool("GetKasizukiMaidLikability()==11 && GetKasizukiMaidWorkCount()==2")?);
        t.exec("guest","SetKasizukiSystemFlag('男種類',20);SetKasizukiMaidFlag(0,'好感度＿傅き男１',23);SetKasizukiSystemFlag('counter','2147483647');AddKasizukiSystemFlag('counter',1);")?;
        assert!(t.eval_bool("IsKasizukiGuest() && GetKasizukiMaidLikability()==23 && GetKasizukiSystemFlag('counter')==='-2147483648'")?);
        t.exec("reset","SetKasizukiSystemFlag('counter','none');AddKasizukiSystemFlag('counter',3); SetMusicRelease(5);SetMusicRelease(2);")?;
        assert!(
            t.eval_bool("GetKasizukiSystemFlag('counter')==='3' && GetManFlag('_楽曲解放')==5")?
        );
        t.runtime.host_mut().state.player.days = 4;
        assert!(
            t.eval_bool("BjDayFirst() && BjDayFirst('ブラックジャック終了') && !BjDayFirst()")?
        );
        assert!(t.eval("PresetLoadNPC()").is_err());
        assert!(matches!(t.eval("SetSexualParam(1,2)")?, Variant::Void));
        assert!(t.eval("GetKasizukiMaidFlag(42,'x')").is_err());
        Ok(())
    }

    #[test]
    fn standalone_character_flags_and_status_follow_legacy_scope() -> Result<()> {
        let mut t = script()?;
        t.exec("status","SetMaidFlag('route',7); SetMaidCondition(0,'信頼'); AddMaidStatus(0,'好感度',450); SetContractType(0,'専属');")?;
        assert!(t.eval_bool("GetMaidStatus(0,'好感度')==460 && GetMaidStatus(0,'関係')=='信頼' && GetMaidStatus(0,'契約タイプ')=='専属' && GetMaidFlag('route')==7")?);
        t.runtime
            .host_mut()
            .state
            .retail
            .characters
            .get_mut("character-0")
            .unwrap()
            .old = Some(Default::default());
        t.eval("SetCompatibilityMode(1)")?;
        assert!(t.eval_bool("GetMaidFlag('route')==0 && GetMaidStatus(0,'好感度')==299 && SetMaidFlag(0,'route',3)==3")?);
        t.eval("SetCompatibilityMode(0)")?;
        assert!(t.eval_bool("GetMaidFlag('route')==7")?);
        assert!(t.eval("SetContractType(0,'unknown')").is_err());
        Ok(())
    }

    #[test]
    fn standalone_script_status_without_legacy_game_tables() -> Result<()> {
        let mut t = script()?;
        assert!(t.eval("GetTmpFlag()").is_err());
        assert!(t.eval("SetTmpFlag('a')").is_err());
        assert!(t.eval_bool("GetTmpFlag('a')==0 && SetTmpFlag('a',-2)==-2 && GetTmpFlag('a')==-2")?);
        t.exec("state", "SetSeikeikenFront(0,1);SetSeikeikenBack(0,1);SetSeikeikenFront(0,0);")?;
        assert_eq!(t.runtime.host().state.maid(0).unwrap().seikeiken, 2);
        assert!(t.eval_bool("GetYotogiSkillLevel(0,1234)==0")?);
        assert!(t.eval("GetYotogiSkillLevel(0)").is_err());
        assert!(t.eval("IsSeikantai(0,'口')").is_err());
        t.eval("SetCompatibilityMode(1)")?;
        assert!(t.eval("IsSeikantai(0,'口')").is_err());
        let state = &mut t.runtime.host_mut().state;
        let old = state.retail.characters.entry("character-0".into()).or_default().old.get_or_insert_with(Default::default);
        old.sexual.mouth = 1000;
        old.sexual.throat = 999;
        assert!(t.eval_bool("IsSeikantai(0,'口') && !IsSeikantai(0,'喉')")?);
        assert!(t.eval("IsSeikantai(0,'unknown')").is_err());
        Ok(())
    }

    #[test]
    fn standalone_schedule_and_menu_use_active_slot() -> Result<()> {
        let mut t = script()?;
        let s = &mut t.runtime.host_mut().state;
        s.player.schedule_slot[39].maid_guid = "character-1".into();
        s.player.schedule_slot[39].noon_success_level = 4;
        let b = s.bodies.entry(1).or_default();
        b.props.insert("head".into(), (3, 7));
        b.menu_files.insert("head".into(), "face.menu".into());
        b.props.insert("sintyou".into(), (1, 30));
        assert!(t.eval_bool("GetNoonScuccessLevel(0)==-1 && GetNoonScuccessLevel(1)==4 && GetMenuFileNameFromMaidMPN(1,'head')=='face.menu' && GetMaidMPN(1,'35')=='head' && GetMaidMPN(1,'sintyou')==30 && GetMenuFileNameFromMaidMPN(1,'sintyou')==''")?);
        assert!(t.eval("GetMenuFileNameFromMaidMPN('1','head')").is_err());
        t.exec("roles","SetMaidLeader(1,1);SetMainCharaFlag(1,1);SetInitSeikeiken(1,'No_Yes');SetInitSeikeiken(1,'bad');")?;
        let s = &t.runtime.host().state;
        assert!(s.maid(1).unwrap().leader && s.retail.main_characters.contains(&1));
        assert_eq!(s.maid(1).unwrap().init_seikeiken, 2);
        Ok(())
    }

    #[test]
    #[cfg(feature = "runtime")]
    fn compatibility_returns_to_the_saved_adv_depth() -> Result<()> {
        use crate::kag::AdvKag;
        use crysirror_core::{state::ClientId, vfs::MemoryProvider};
        let tmp = tempfile::tempdir()?;
        let mut vfs = crysirror_runtime::vfs::DiskVfs::new(tmp.path(), &tmp.path().join("out"))?;
        let id = ClientId(1);
        vfs.register(id, "com3d2", "test")?;
        let mut fs = MemoryProvider::new();
        fs.insert(
            "scope.ks",
            br#"@eval exp="SetCompatibilityMode(1)"
@call label="*sub"
@eval exp="SetManFlag('normal', IsExistentFile('scope.ks'))"
@s
*sub
@eval exp="SetTmpFlag('inside', !IsExistentFile('scope.ks'))"
@eval exp="SetManFlag('protected', 1)"
@return
"#
            .to_vec(),
        )?;
        vfs.mount_provider(id, "fixture", Arc::new(fs))?;
        let storage = vfs.storage(id)?;
        let files = Arc::new(GameFiles::index(storage.as_ref(), &["fixture".into()])?);
        let mut state = TjsState::default();
        state
            .retail
            .attach(files.clone(), storage.clone(), "data")?;
        let mut vm = AdvKag::for_profile(&files, storage.as_ref(), ScriptProfile::Com3d2, state)?;
        vm.load(&files, storage.as_ref(), "scope", "")?;
        vm.exec();
        vm.update(16, false, false, &files, storage.as_ref())?;
        let mut t = vm.tjs.lock().unwrap();
        assert!(t.eval_bool(
            "GetTmpFlag('inside') && GetManFlag('normal') && !GetManFlag('protected')"
        )?);
        assert!(t.eval("GetRentalNPCName()").is_err());
        t.eval("SetManFlag('2.0モード',1)")?;
        assert!(t.eval_bool("GetRentalNPCName()=='' && !IsMarriage(0) && !IsNewWifeFlag(0)")?);
        Ok(())
    }

    #[test]
    #[cfg(feature = "runtime")]
    #[ignore = "requires COM3D2_ROOT standalone installation"]
    fn installed_standalone_tjs_tables() -> Result<()> {
        let (_tmp, mut t) = installed_tjs()?;
        assert!(t.eval_bool("IsExistentFile('FACILITY.NEI') && !IsExistentFileLegacy('facility.nei') && !IsPluginPack('VP001')")?);
        assert!(t.eval_bool("IsBuildingFacility('トレーニングルーム') && !IsBuildingFacilityUpward('トレーニングルーム')")?);
        assert!(t.eval("IsBuildingFacility('no such facility')").is_err());
        t.exec("tables","SetKasizukiSystemFlag('選択部屋',3000);SetKasizukiMaidFlag(0,'仕事回数',7);SetKasizukiPassFlag(1000);SetFeature(0,'元気な笑顔',0);")?;
        assert!(t.eval_bool("GetKasizukiSelectRoomName()=='SM' && GetKasizukiMaidWorkCountName()=='慣れ始め' && GetKasizukiMaidLikabilityName()=='お近づき' && GetKasizukiSystemFlag('プレイ実行＿1000')==='True' && IsMaidFeature(0,'元気な笑顔')")?);
        assert!(t.eval("SetKasizukiPassFlag(-5)").is_err());
        let award = {
            let retail = &mut t.runtime.host_mut().state.retail;
            retail.index_parts()?;
            let awards = retail.table("scenario_get_item_list.nei")?;
            let row=(1..awards.rows).find(|&r|awards.exists(0,r)).context("installed scenario award")?;
            format!("{}.menu",awards.string(0,row).to_lowercase())
        };
        assert!(!t.runtime.host().state.retail.owned_parts[&award]);
        let acquire=format!("SetNewItem('{}')",award.to_uppercase());
        t.runtime.host_mut().state.retail.compatibility=true;
        t.eval(&acquire)?;
        assert!(!t.runtime.host().state.retail.owned_parts[&award]);
        t.runtime.host_mut().state.retail.compatibility=false;
        t.eval(&acquire)?;
        assert!(t.runtime.host().state.retail.owned_parts[&award]);
        assert!(t.eval("SetNewItem('not-an-installed-award.menu')").is_err());
        t.eval("PresetLoad(0,'pre_maid_secretary')")?;
        assert!(t.eval_bool("GetMaidMPN(0,'sintyou') >= 20 && GetMaidMPN(0,'sintyou') <= 80")?);
        assert!(
            t.runtime.host().state.retail.appearances[&0]
                .colors
                .is_some()
        );
        t.eval("PresetLoad(0,'pre_muku')")?;
        assert!(t.eval_bool("GetMaidMPN(0,'sintyou')==27 && GetMenuFileNameFromMaidMPN(0,'head')=='face007_i_.menu' && GetMenuFileNameFromMaidMPN(0,'hairf')=='hair_f090_i_.menu'")?);
        let before = t.eval("GetMaidMPN(0,'sintyou')")?;
        assert!(t.eval("PresetLoad(0,'unknown')").is_err());
        assert_eq!(t.eval("GetMaidMPN(0,'sintyou')")?, before);
        assert!(!t.eval_bool("IsJobMaid(0,'Novice2')")?);
        let c = t
            .runtime
            .host_mut()
            .state
            .retail
            .characters
            .entry("character-0".into())
            .or_default();
        c.job_levels.extend([(10, 0), (70, 4)]);
        assert!(t.eval_bool(
            "IsJobMaid(0,'Novice2') && GetJobMaidLevel(0,'Empire')==4 && !IsJobMaid(1,'Empire')"
        )?);
        assert!(t.eval("IsMaidClass(0,'Empire')").is_err());
        assert!(t.eval("IsJobMaid('0','Empire')").is_err());
        assert!(!t.eval_bool("CasinoShopBuy(20)")?);
        activities::buy_for_test(&mut t.runtime.host_mut().state.retail, 20);
        assert!(
            t.eval_bool("CasinoShopBuy(20,20) && !CasinoShopBuy(20,30) && !CasinoShopBuy(-1)")?
        );
        {
            let s = &mut t.runtime.host_mut().state;
            s.player.schedule_slot[0].maid_guid = "character-1".into();
            s.player.schedule_slot[39].maid_guid = "character-0".into();
            s.maids.get_mut(&0).unwrap().noon_work_id = 2201;
            s.maids.get_mut(&1).unwrap().noon_work_id = 2202;
            s.retail
                .characters
                .entry("character-1".into())
                .or_default()
                .communication[0] = true;
            s.retail.facilities.push(facility::Facility {
                type_id: 120,
                assigned: [BTreeSet::from(["character-1".into()]), BTreeSet::new()],
                costume: facility::Costume::Default,
            });
        }
        assert!(t.eval_bool("SetCommunicationChara()")?);
        assert_eq!(t.runtime.host().state.active_maids, [Some(1), Some(0)]);
        t.eval("SetMaidCostumeFacility(0)")?;
        let appearance = &t.runtime.host().state.retail.appearances[&1];
        assert!(appearance.slots.values().any(|p| p.temp.is_some()));
        t.runtime.host_mut().state.retail.facilities[0].costume = facility::Costume::Edited(
            BTreeMap::from([("wear".into(), Some("custom.menu".into()))]),
        );
        t.eval("SetMaidCostumeFacility(0)")?;
        assert_eq!(
            t.runtime.host().state.retail.appearances[&1].slots["wear"]
                .temp
                .as_ref()
                .unwrap()
                .0,
            "custom.menu"
        );
        t.eval("ResetMaidCostumeFacility(0)")?;
        assert!(
            t.runtime.host().state.retail.appearances[&1].slots["wear"]
                .temp
                .is_none()
        );
        t.runtime.host_mut().state.active_maids.swap(0, 1);
        {
            let s = &mut t.runtime.host_mut().state;
            for m in s.maids.values_mut() {
                m.personal = 80;
                m.likability = 0;
            }
            s.retail.events = Default::default();
        }
        t.eval("GetEventAdded()")?;
        assert!(!t.eval_bool("GetEventAdded()")?);
        t.eval("AddMaidStatus(0,'好感度',5)")?;
        assert!(t.eval_bool("GetEventAdded()")?);
        assert!(t.runtime.host().state.retail.events.added.contains(&10));
        assert!(!t.eval_bool("GetEventAdded()")?);
        t.eval("SetEventEndFlag(0,10,1)")?;
        t.eval("GetEventAdded()")?;
        t.eval("SetEventEndFlag(0,10,0)")?;
        assert!(t.eval_bool("GetEventAdded()")?);
        t.runtime
            .host_mut()
            .state
            .maids
            .get_mut(&0)
            .unwrap()
            .likability = 10;
        let tag = BTreeMap::from([
            ("name".into(), "SceneVRCommunication".into()),
            ("karaoke".into(), "false".into()),
        ]);
        assert!(t.eval("IsVRKaraokeMode()").is_err());
        t.runtime
            .host_mut()
            .state
            .retail
            .scene_loaded("SceneVRCommunication", &tag);
        assert!(t.eval_bool("IsVRKaraokeMode()")?);
        t.runtime
            .host_mut()
            .state
            .retail
            .scene_loaded("SceneYotogi", &BTreeMap::new());
        assert!(t.eval("IsVRKaraokeMode()").is_err());
        t.runtime
            .host_mut()
            .state
            .retail
            .selection
            .as_mut()
            .unwrap()
            .selected = vec![Some((10510, None)), Some((10, Some(10)))];
        assert!(t.eval_bool("GetYotogiSkillSpecialType()=='Null' && IsSelectedYotogiSkill(10) && !IsSelectedYotogiSkill(10510) && IsYotogiSubCharaSelect()")?);
        t.runtime.host_mut().state.stock.pop();
        assert!(!t.eval_bool("IsYotogiSubCharaSelect()")?);
        t.runtime.host_mut().state.stock.push(1);
        t.runtime
            .host_mut()
            .state
            .retail
            .scene_loaded("SceneADV", &tag);
        assert!(matches!(
            t.eval("GetYotogiSkillSpecialType()")?,
            Variant::Void
        ));
        assert!(matches!(
            t.eval("IsSelectedYotogiSkill(10)")?,
            Variant::Void
        ));
        // Row 1010 requires an eligible experience state, trust and two independent stats.
        t.runtime
            .host_mut()
            .state
            .maids
            .get_mut(&0)
            .unwrap()
            .seikeiken = 1;
        t.exec(
            "conditions",
            "SetMaidCondition(0,'信頼');AddMaidStatus(0,'淫欲',3000);",
        )?;
        assert!(!t.eval_bool("IsKasizukiCorrectSenarioFlag(0,1010)")?);
        t.eval("AddMaidStatus(0,'奉仕',3000)")?;
        assert!(t.eval_bool("IsKasizukiCorrectSenarioFlag(0,1010)")?);
        t.eval("SetMaidCondition(0,'恋人')")?;
        assert!(!t.eval_bool("IsKasizukiCorrectSenarioFlag(0,1010)")?);
        Ok(())
    }
    #[cfg(feature="runtime")]
    fn installed_tjs() -> Result<(tempfile::TempDir,Tjs)> {
        use crysirror_core::state::ClientId;
        use crysirror_resource::{ArchiveMount, DecodeContext, FormatRegistry};
        let root =
            std::path::PathBuf::from(std::env::var_os("COM3D2_ROOT").context("COM3D2_ROOT")?);
        let tmp = tempfile::tempdir()?;
        let mut disk = crysirror_runtime::vfs::DiskVfs::new(&root, tmp.path())?;
        let id = ClientId(1);
        disk.register(id, "com3d2", "test")?;
        let mut reg = FormatRegistry::default();
        crysirror_formats_arc::register(&mut reg)?;
        let roots: Vec<String> = ["csv", "parts", "parts2", "system", "system2"]
            .into_iter()
            .map(String::from)
            .collect();
        for name in &roots {
            let path = VfsPath::new(Mount::Asset, &format!("GameData/{name}.arc"))?;
            let reader = disk.storage(id)?.open_asset(&path)?;
            let cx = DecodeContext::new(id);
            let (_, archive) = reg.open(reader, Some("kiss/warc"), &cx)?;
            disk.mount_provider(id, name, Arc::new(ArchiveMount::new(archive, cx)?))?;
        }
        let storage = disk.storage(id)?;
        let files = Arc::new(GameFiles::index(storage.as_ref(), &roots)?);
        let mut t = script()?;
        t.runtime
            .host_mut()
            .state
            .retail
            .attach(files, storage, "COM3D2x64_Data")?;
        Ok((tmp,t))
    }

    #[test]
    #[cfg(feature = "runtime")]
    #[ignore = "requires COM3D2_ROOT standalone installation"]
    fn standalone_trophies_follow_contract_and_relation() -> Result<()> {
        let (_tmp, mut vm) = installed_tjs()?;
        let state = &mut vm.runtime.host_mut().state;
        for (id, contract) in [(0, 1), (1, 2)] {
            let maid = state.maids.get_mut(&id).unwrap();
            maid.contract_type = contract;
            state.retail.characters.entry(maid.guid_id.clone()).or_default().relation = 2;
        }
        state.retail.characters.get_mut("character-1").unwrap().sub_name = Some("fixture".into());
        state.retail.compatibility = true;
        vm.exec("linked", "SetNewTrophy(40);CheckTrophyContractType();")?;
        assert!(!vm.runtime.host().state.player.have_trophy.contains(&40));
        vm.runtime.host_mut().state.retail.compatibility = false;
        vm.exec("stock", "CheckTrophyContractType();CheckTrophyContractType();SetNewTrophy(-1);")?;
        let state = &vm.runtime.host().state;
        assert!(state.player.have_trophy.contains(&10));
        assert!(state.player.have_trophy.contains(&70));
        assert!(!state.player.have_trophy.contains(&40));
        assert!(!state.player.have_trophy.contains(&20));
        assert_eq!(state.requests.iter().filter(|r| **r == crate::tjs::Request::TrophyEffect(10)).count(), 1);
        vm.eval("SetNewTrophy(40)")?;
        assert!(vm.runtime.host().state.player.have_trophy.contains(&40));
        Ok(())
    }

    #[test]
    #[cfg(feature="runtime")]
    #[ignore="requires COM3D2_ROOT standalone installation"]
    fn standalone_scheduled_cast_and_activation() -> Result<()> {
        let (_tmp,mut t)=installed_tjs()?;
        let s=&mut t.runtime.host_mut().state;
        s.player.schedule_slot[0].maid_guid="character-1".into();
        s.player.schedule_slot[39].maid_guid="character-0".into();
        s.maids.get_mut(&1).unwrap().noon_work_id=2202;
        s.maids.get_mut(&0).unwrap().noon_work_id=2001;
        s.maids.get_mut(&1).unwrap().night_work_id=10001;
        s.maids.get_mut(&0).unwrap().night_work_id=10000;
        assert!(t.eval("IsCommunicationChara()").is_err());
        assert!(!t.eval_bool("SetCommunicationChara()")?);
        t.eval("SetRandomCommuChara()")?;
        assert_eq!(t.runtime.host().state.active_maids[0],Some(0));
        t.eval("CreateYotogiCharaList()")?;
        assert!(!t.eval_bool("IsSelectChara()")?);
        assert!(t.eval_bool("IsReceptionChara()")?);
        t.eval("SetRandomReceptionChara()")?;
        assert_eq!(t.runtime.host().state.active_maids,[Some(1),None]);
        t.runtime.host_mut().state.player.generic_flag.insert("時間帯".into(),3);
        t.eval("CreateYotogiCharaList()")?;
        assert_eq!(t.runtime.host().state.chara_guid_list,["character-0"]);
        t.eval("CreateVipCharaList()")?;
        assert!(!t.eval_bool("IsSelectChara()")?);
        t.eval("SetRandomCommuChara()")?;
        assert_eq!(t.runtime.host().state.active_maids[0],None);
        assert_eq!(t.runtime.host().state.requests.last(),Some(&crate::tjs::Request::DeactivateMaid(0)));
        t.eval("SetMaidRanking1st()")?;
        assert_eq!(t.runtime.host().state.active_maids[0],Some(1));
        t.runtime.host_mut().state.maids.get_mut(&0).unwrap().popular_rank=1;
        t.eval("SetMaidRanking1st()")?;
        assert_eq!(t.runtime.host().state.active_maids[0],Some(0));
        assert!(t.eval_bool("CreateStockMaidLoopData()==2")?);
        t.eval("SetActiveSlotFromStockMaidLoopData(0)")?;
        assert_eq!(t.runtime.host().state.requests.last(),Some(&crate::tjs::Request::SetActiveMaid{slot:0,chara:1,visible:false}));
        Ok(())
    }

    #[test]
    #[cfg(feature="runtime")]
    #[ignore="requires COM3D2_ROOT standalone installation"]
    fn standalone_stock_creation_and_live_presets() -> Result<()> {
        let (_tmp,mut t)=installed_tjs()?;
        t.runtime.host_mut().state.player.days=1;
        for (name,preset) in [("Muku","pre_muku"),("Majime","pre_mazime"),("Rindere","pre_rindere")] {
            t.eval(&format!("CreateNewMaid('{name}')"))?;
            let s=&t.runtime.host().state;
            let id=*s.stock.last().unwrap();
            assert_eq!(s.active_maids[0],Some(id));
            let m=&s.maids[&id];
            assert_eq!(m.employment_day,1);
            assert!(!m.guid_id.is_empty() && !m.first_name.is_empty() && !m.profile.is_empty());
            assert!(m.body.height>0 && !m.skill_data.is_empty());
            assert_eq!(m.current_maid_class,10);
            assert_eq!(m.current_yotogi_class,460);
            assert_eq!(m.cur_hp,100);
            t.eval(&format!("PresetLoad(0,'{preset}')"))?;
            let s=&t.runtime.host().state;
            let model=&s.retail.stock_models[&id];
            assert!(model.visible && model.slots.values().any(|s|s.model.is_some()));
            assert!(s.bodies[&id].loaded_slots.contains("body"));
        }
        t.eval("CreateNewMaid('紗姫')")?;
        assert!(t.eval_bool("GetMaidStatus(0,'名前')=='紗姫' && GetMaidStatus(0,'ヒロインタイプ')=='サブ'")?);
        let id=t.runtime.host().state.active_maids[0].unwrap();
        let st=&t.runtime.host().state;
        assert!(st.retail.characters[&st.maids[&id].guid_id].portraits.is_some());
        let count=st.stock.len();
        let next=st.next_character;
        assert!(t.eval("CreateNewMaid('not a stock definition')").is_err());
        let s=&t.runtime.host().state;
        assert_eq!(s.stock.len(),count);
        assert_eq!(s.next_character,next);
        assert_eq!(s.active_maids[0],Some(id));
        Ok(())
    }

}
