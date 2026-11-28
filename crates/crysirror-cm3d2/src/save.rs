// SPDX-License-Identifier: MPL-2.0
//! Original save files (`SaveData/SaveDataNNN.save`): `GameMain.Serialize`
//! / `Deserialize` with `CharacterMgr`, `Maid` and `ScriptManager` sections
//! in their `BinaryWriter` encoding. Slot −1 (`SaveData-01.save`) is the
//! title's return point.
pub mod facility_v5;
pub(crate) mod management_1040;
pub mod player_1040;
pub mod slot_1040;
pub(crate) mod status_1040;
pub(crate) mod stock_1040;

use crate::{
    adapter::{Game, Opener},
    binio::{Reader, Writer},
    chara::Maid,
    kag::AdvKag,
    maid_param::{MaidParam, PlayerParam},
    params::MaidStatus,
};
use anyhow::{ensure, Context, Result};
use crysirror_core::path::{Mount, VfsPath};

/// `Maid.Initialize` `AddProp` order: the `m_dicMaidProp` order written by
/// `SerializeProp`.
const PROP_ORDER: [&str; 86] = [
    "null_mpn", "MuneL", "MuneTare", "RegFat", "ArmL", "Hara", "RegMeet", "KubiScl", "UdeScl", "EyeScl", "EyePosX", "EyePosY", "HeadX", "HeadY",
    "DouPer", "sintyou", "koshi", "kata", "west", "MuneUpDown", "MuneYori", "body", "head", "hairf", "hairr", "hairt", "hairs", "wear", "skirt",
    "mizugi", "bra", "panz", "stkg", "shoes", "headset", "glove", "acchead", "hairaho", "accha", "acchana", "acckamisub", "acckami", "accmimi",
    "accnip", "acckubi", "acckubiwa", "accheso", "accude", "accashi", "accsenaka", "accshippo", "accanl", "accvag", "megane", "accxxx", "handitem",
    "acchat", "haircolor", "skin", "acctatoo", "underhair", "hokuro", "mayu", "lip", "eye", "eye_hi", "chikubi", "chikubicolor", "moza", "onepiece",
    "set_maidwear", "set_mywear", "set_underwear", "folder_eye", "folder_mayu", "folder_underhair", "folder_skin", "kousoku_upper", "kousoku_lower",
    "seieki_naka", "seieki_hara", "seieki_face", "seieki_mune", "seieki_hip", "seieki_ude", "seieki_ashi",
];

/// `GameMain.SerializeHeader`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Header {
    pub save_time: String,
    pub game_day: i32,
    pub player_name: String,
    pub maid_num: i32,
    pub comment: String,
}

/// `GameMain.MakeSavePathFileName`.
pub fn save_path(slot: i32) -> Result<VfsPath> {
    Ok(VfsPath::new(Mount::Save, &format!("SaveData/SaveData{slot:03}.save"))?)
}

/// .NET `String.GetHashCode` of Unity 4's Mono (`h = 31·h + c` over UTF-16
/// units, two per step): `MaidProp.nFileNameRID` of the lower-cased file.
pub fn mono_string_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| h.wrapping_shl(5).wrapping_sub(h).wrapping_add(i32::from(c)))
}

pub(crate) fn write_header(w: &mut Writer, h: &Header) {
    w.str(&h.save_time);
    w.i32(h.game_day);
    w.str(&h.player_name);
    w.i32(h.maid_num);
    w.str(&h.comment);
}

fn read_header(r: &mut Reader) -> Result<Header> {
    Ok(Header { save_time: r.str()?, game_day: r.i32()?, player_name: r.str()?, maid_num: r.i32()?, comment: r.str()? })
}

/// `GameMain.GetSaveDataHeader`.
pub fn header(bytes: &[u8]) -> Result<Header> {
    let mut r = Reader::new(bytes);
    r.expect_str("CM3D2_SAVE").context("セーブデータファイルのヘッダーが不正です。")?;
    r.i32()?;
    read_header(&mut r)
}

/// `Maid.SerializeProp`.
fn write_props(w: &mut Writer, m: &Maid) -> Result<()> {
    w.str("CM3D2_MPROP_LIST");
    w.i32(100);
    w.i32(PROP_ORDER.len() as i32);
    for name in PROP_ORDER {
        let idx = crate::chara::mpn(name).with_context(|| format!("MPN {name}"))?;
        let p = &m.props[idx];
        w.str(name);
        // MaidProp.Serialize.
        w.str("CM3D2_MPROP");
        w.i32(100);
        w.i32(idx as i32);
        w.str(name);
        w.i32(i32::from(p.kind));
        w.i32(p.default);
        w.i32(p.value);
        // value_LinkMAX: set only for props named …上限 (none in 1.00).
        w.i32(0);
        w.str(&p.file);
        w.i32(if p.kind == 3 { mono_string_hash(&p.file.to_lowercase()) } else { 0 });
        w.bool(p.dut);
        w.i32(p.max);
        w.i32(p.min);
    }
    Ok(())
}

/// `Maid.DeserializeProp` (the caller runs `AllProcPropSeqStart`).
fn read_props(r: &mut Reader, m: &mut Maid) -> Result<()> {
    r.expect_str("CM3D2_MPROP_LIST").context("メイドプロパティリストのヘッダーが不正です。")?;
    r.i32()?;
    let n = r.i32()?;
    ensure!((0..=1024).contains(&n), "MaidProp count {n}");
    for _ in 0..n {
        let key = r.str()?;
        r.expect_str("CM3D2_MPROP").context("メイドプロパティのヘッダーが不正です。")?;
        r.i32()?;
        let _idx = r.i32()?;
        let _name = r.str()?;
        let kind = r.i32()?;
        let default = r.i32()?;
        let value = r.i32()?;
        let _link_max = r.i32()?;
        let file = r.str()?;
        let _rid = r.i32()?;
        let _dut = r.bool()?;
        let (max, min) = (r.i32()?, r.i32()?);
        let i = crate::chara::mpn(&key).with_context(|| format!("指定されたプロパティ名がありません。{key}"))?;
        let p = &mut m.props[i];
        p.kind = u8::try_from(kind).context("MaidProp type")?;
        (p.default, p.value, p.max, p.min) = (default, value, max, min);
        p.file = file;
        p.temp_file.clear();
        // MaidProp.Deserialize forces boDut.
        p.dut = true;
    }
    Ok(())
}

/// `MaidParts.Serialize`.
fn write_colors(w: &mut Writer, m: &Maid) {
    w.str("CM3D2_MULTI_COL");
    w.i32(100);
    w.i32(m.parts_color.len() as i32);
    for c in &m.parts_color {
        w.bool(c.used);
        for v in [c.main_hue, c.main_chroma, c.main_brightness, c.main_contrast, c.shadow_rate, c.shadow_hue, c.shadow_chroma, c.shadow_brightness, c.shadow_contrast] {
            w.i32(v);
        }
    }
}

fn read_colors(r: &mut Reader, m: &mut Maid) -> Result<()> {
    r.expect_str("CM3D2_MULTI_COL").context("無限色のヘッダーが不正です。")?;
    r.i32()?;
    let n = r.i32()?;
    ensure!(n as usize == m.parts_color.len(), "無限色数が異なります。セーブデータが破損しています。");
    for i in 0..m.parts_color.len() {
        let used = r.bool()?;
        let mut v = [0i32; 9];
        for x in &mut v {
            *x = r.i32()?;
        }
        let c = crate::parts_color::PartsColor {
            used,
            main_hue: v[0],
            main_chroma: v[1],
            main_brightness: v[2],
            main_contrast: v[3],
            shadow_rate: v[4],
            shadow_hue: v[5],
            shadow_chroma: v[6],
            shadow_brightness: v[7],
            shadow_contrast: v[8],
        };
        m.set_parts_color(i, c);
    }
    Ok(())
}

/// `Maid.SerializeMisc`. The thumbnail icon (`ThumShot`) is not rendered by
/// this port, so none is written (SATD-cm3d2-game-logic).
fn write_misc(w: &mut Writer, m: &Maid, slot: i32) {
    w.str("CM3D2_MAID_MISC");
    w.i32(100);
    w.i32(slot);
    w.i32(0);
    w.str("");
    for v in m.man_color {
        w.f32(v);
    }
}

/// `Maid.DeserializeMisc`: active slot number.
fn read_misc(r: &mut Reader, m: &mut Maid) -> Result<i32> {
    r.expect_str("CM3D2_MAID_MISC").context("メイドその他のヘッダーが不正です。")?;
    let version = r.i32()?;
    let slot = r.i32()?;
    let icon = r.i32()?;
    ensure!((0..=64 << 20).contains(&icon), "maid icon size {icon}");
    r.take(icon as usize)?;
    if 8 <= version {
        r.str()?;
    }
    for c in &mut m.man_color {
        *c = r.f32()?;
    }
    Ok(slot)
}

impl Game {
    fn stock(&self, man: bool) -> Vec<usize> {
        self.characters.iter().enumerate().filter(|(_, c)| c.man == man && c.is_stock()).map(|(i, _)| i).collect()
    }

    fn active_slot(&self, man: bool, index: usize) -> i32 {
        let slots: &[Option<usize>] = if man { &self.active_men } else { &self.active_maids };
        slots.iter().position(|s| *s == Some(index)).map_or(-1, |p| p as i32)
    }

    /// `GameMain.Serialize(slot, comment)` into bytes.
    pub fn serialize_save(&self, comment: &str) -> Result<Vec<u8>> {
        let tjs = self.adv.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let state = &tjs.runtime.host().state;
        let (maids, men) = (self.stock(false), self.stock(true));
        let mut w = Writer::default();
        w.str("CM3D2_SAVE");
        w.i32(100);
        let h = Header { save_time: crate::adapter::save_time(), game_day: state.player.days, player_name: state.player.player_name.clone(), maid_num: maids.len() as i32, comment: comment.into() };
        write_header(&mut w, &h);
        // CharacterMgr.Serialize.
        w.str("CM3D2_CHR_MGR");
        w.i32(100);
        PlayerParam::write(&state.player, &mut w);
        w.i32(men.len() as i32);
        for &i in &men {
            write_props(&mut w, &self.characters[i])?;
            write_misc(&mut w, &self.characters[i], self.active_slot(true, i));
        }
        w.i32(maids.len() as i32);
        for &i in &maids {
            let c = &self.characters[i];
            write_props(&mut w, c)?;
            write_colors(&mut w, c);
            MaidParam::write(state.maids.get(&i).context("stock maid parameters")?, &mut w);
            write_misc(&mut w, c, self.active_slot(false, i));
        }
        drop(tjs);
        // ScriptManager.Serialize → BaseKagManager.Serialize of adv_kag.
        w.str("CM3D2_SCRIPT");
        w.i32(100);
        w.str("CM3D2_KAG");
        w.i32(100);
        let blob = self.adv.serialize_blob()?;
        w.i32(blob.len() as i32);
        w.bytes.extend_from_slice(&blob);
        w.bool(self.adv.fade_wait());
        w.bool(self.adv.enabled);
        Ok(w.bytes)
    }

    /// `GameMain.Serialize`: write the slot file.
    pub fn save_slot(&self, opener: &Opener, slot: i32, comment: &str) -> Result<()> {
        let bytes = self.serialize_save(comment)?;
        opener.storage.write_atomic(&save_path(slot)?, &bytes)?;
        Ok(())
    }

    /// `GameMain.Deserialize`: restore characters, parameters and the ADV
    /// KAG, then `Exec` it at its label. The data is decoded completely
    /// before live state changes; a malformed file leaves the game as it was.
    pub fn load_slot(&mut self, opener: &mut Opener, slot: i32) -> Result<()> {
        let bytes = opener.storage.read(&save_path(slot)?).with_context(|| format!("ファイルを開けません。SaveData{slot:03}.save"))?;
        self.deserialize_save(opener, &bytes)
    }

    pub fn deserialize_save(&mut self, opener: &mut Opener, bytes: &[u8]) -> Result<()> {
        let mut r = Reader::new(bytes);
        r.expect_str("CM3D2_SAVE").context("セーブデータファイルのヘッダーが不正です。")?;
        r.i32()?;
        read_header(&mut r)?;
        r.expect_str("CM3D2_CHR_MGR").context("セーブデータファイルのヘッダーが不正です。")?;
        r.i32()?;
        let data = { self.adv.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?.runtime.host().state.data()? };
        let mut player = crate::params::PlayerStatus::default();
        PlayerParam { st: &mut player, data: &data }.read(&mut r)?;
        let men = self.stock(true);
        let n = r.i32()?;
        ensure!(n as usize == men.len(), "男スロット数が不正です。");
        let mut man_data = Vec::new();
        for &i in &men {
            let mut m = self.characters[i].clone();
            read_props(&mut r, &mut m)?;
            let slot = read_misc(&mut r, &mut m)?;
            man_data.push((i, m, slot));
        }
        let n = r.i32()?;
        ensure!((0..=1000).contains(&n), "stock maid count {n}");
        let mut maid_data = Vec::new();
        for _ in 0..n {
            let mut m = Maid::new(false);
            read_props(&mut r, &mut m)?;
            read_colors(&mut r, &mut m)?;
            let mut st = MaidStatus::default();
            MaidParam { st: &mut st, data: &data }.read(&mut r)?;
            let slot = read_misc(&mut r, &mut m)?;
            maid_data.push((m, st, slot));
        }
        r.expect_str("CM3D2_SCRIPT").context("SCRIPTのヘッダーが不正です。")?;
        r.i32()?;
        r.expect_str("CM3D2_KAG").context("KAGのヘッダーが不正です。")?;
        r.i32()?;
        let n = r.i32()?;
        ensure!(n >= 0, "KAG data size");
        let blob = r.take(n as usize)?.to_vec();
        let (fade_wait, enabled) = (r.bool()?, r.bool()?);
        // Restore the script first: a failure leaves the characters as they were.
        let storage = opener.storage.clone();
        let mut adv = AdvKag::new(&self.files, storage.as_ref())?;
        adv.tjs = self.adv.tjs.clone();
        adv.deserialize_blob(&self.files, storage.as_ref(), &blob)?;
        adv.set_fade_wait(fade_wait);
        adv.enabled = enabled;
        // CharacterMgr.Deserialize: deactivate, banish the stock maids, men
        // in place, the saved maids appended in order.
        self.stop_motion_scripts();
        for c in &mut self.characters {
            c.visible = false;
        }
        self.active_maids = [None; 18];
        self.active_men = [None; 6];
        let stock_maids = self.stock(false);
        let mut kept = Vec::new();
        let mut remap = std::collections::HashMap::new();
        for (i, c) in std::mem::take(&mut self.characters).into_iter().enumerate() {
            if !stock_maids.contains(&i) {
                remap.insert(i, kept.len());
                kept.push(c);
            }
        }
        self.characters = kept;
        for (i, m, slot) in man_data {
            let k = remap[&i];
            self.characters[k] = m;
            if let Some(s) = usize::try_from(slot).ok().and_then(|s| self.active_men.get_mut(s)) {
                *s = Some(k);
            }
        }
        let mut maid_params = Vec::new();
        for (m, st, slot) in maid_data {
            let k = self.characters.len();
            self.characters.push(m);
            if let Some(s) = usize::try_from(slot).ok().and_then(|s| self.active_maids.get_mut(s)) {
                *s = Some(k);
                self.characters[k].visible = true;
            }
            maid_params.push((k, st));
        }
        {
            let mut tjs = self.adv.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let state = &mut tjs.runtime.host_mut().state;
            state.player = player;
            let old = std::mem::take(&mut state.maids);
            for (i, st) in old {
                if let Some(k) = remap.get(&i) {
                    state.maids.insert(*k, st);
                }
            }
            for (k, st) in maid_params {
                state.maids.insert(k, st);
            }
            state.bodies.clear();
        }
        for id in self.chara_view.reset() {
            self.world.push(serde_json::json!({"op": "despawn", "id": id}));
        }
        let files = self.files.clone();
        for c in &mut self.characters {
            for s in c.slots.values_mut() {
                s.dirty = true;
            }
            c.all_proc_prop(&files, storage.as_ref())?;
            let warnings = std::mem::take(&mut c.warnings);
            self.log.extend(warnings);
        }
        // ScriptManager.Deserialize: a new yotogi KAG and motion KAG 0.
        self.adv = adv;
        self.yotogi_kag = AdvKag::yotogi(self.adv.tjs.clone());
        self.motion_kags.clear();
        self.motion_kags.insert(0, AdvKag::motion(self.adv.tjs.clone()));
        self.backlog.clear();
        self.mirror_tjs()?;
        // Validation only (docs/validation/cm3d2.md): CM3D2_DEBUG_FLAGS=
        // <name>=<int>;… sets player flags, CM3D2_DEBUG_EVAL runs a TJS
        // expression, CM3D2_DEBUG_JUMP=<file>:<label> enters a script label
        // of the loaded state.
        if let Ok(flags) = std::env::var("CM3D2_DEBUG_FLAGS") {
            let mut tjs = self.adv.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let st = &mut tjs.runtime.host_mut().state;
            for (k, v) in flags.split(';').filter_map(|f| f.split_once('=')) {
                // `@rank=<n>`: the first stock maid's popular rank.
                if k == "@rank" {
                    let first = st.stock.first().copied();
                    if let Some(m) = first.and_then(|i| st.maids.get_mut(&i)) {
                        m.popular_rank = v.parse()?;
                    }
                    continue;
                }
                st.player.generic_flag.insert(k.to_owned(), v.parse()?);
            }
        }
        if let Ok(exp) = std::env::var("CM3D2_DEBUG_EVAL") {
            for e in exp.split(';').filter(|e| !e.trim().is_empty()) {
                self.adv.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?.eval(e)?;
            }
            self.run_tjs_requests(opener)?;
        }
        if let Some((file, label)) = std::env::var("CM3D2_DEBUG_JUMP").ok().as_deref().and_then(|v| v.split_once(':')) {
            self.adv.load(&self.files, storage.as_ref(), file, label)?;
        }
        self.adv.exec();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn legacy_colour_record_keeps_seven_entries_and_the_following_field() {
        use super::*;
        let mut maid = Maid::new(false);
        let mut changed = maid.parts_color[2];
        changed.main_hue = 73;
        maid.set_parts_color(2, changed);
        let mut w = Writer::default();
        write_colors(&mut w, &maid);
        w.i32(0x12345678);
        let mut header = Reader::new(&w.bytes);
        header.expect_str("CM3D2_MULTI_COL").unwrap();
        assert_eq!(header.i32().unwrap(), 100);
        assert_eq!(header.i32().unwrap(), 7);
        let mut restored = Maid::new(false);
        let mut r = Reader::new(&w.bytes);
        read_colors(&mut r, &mut restored).unwrap();
        assert_eq!(restored.parts_color, maid.parts_color);
        assert_eq!(r.i32().unwrap(), 0x12345678);
    }

    #[test]
    fn mono_hash_matches_the_managed_string_hash() {
        // h = 31·h + c: "a" → 97, "ab" → 97·31 + 98.
        assert_eq!(super::mono_string_hash(""), 0);
        assert_eq!(super::mono_string_hash("a"), 97);
        assert_eq!(super::mono_string_hash("ab"), 97 * 31 + 98);
    }

    #[test]
    fn save_paths_follow_make_save_path_file_name() {
        assert_eq!(super::save_path(3).unwrap().relative(), "SaveData/SaveData003.save");
        assert_eq!(super::save_path(-1).unwrap().relative(), "SaveData/SaveData-01.save");
    }
}
