// SPDX-License-Identifier: MPL-2.0
//! ScriptManager.Initialize bindings specific to standalone Unity 5.6.4p2.
//! The setup controller and script properties share PlayerStatus's flag dictionary.
use super::{Func, Variant, arg_int, arg_string, boolean, int};
use anyhow::ensure;

/// Standalone ScriptManager.Initialize, in retail registration order. A name
/// here becomes callable only when the host also has an implementation.
pub(super) const REGISTERED: &[&str] = &[
    "GetMaidMPN",
    "GetMenuFileNameFromMaidMPN",
    "SetMaidFlag",
    "SetManFlag",
    "SetTmpFlag",
    "GetMaidFlag",
    "GetManFlag",
    "GetTmpFlag",
    "SetSystemFlag",
    "GetSystemFlag",
    "GetMaidStatus",
    "SetMainCharaFlag",
    "IsSeikantai",
    "CreateNewMaid",
    "PresetLoad",
    "PresetLoadNPC",
    "AddMaidStatus",
    "IsMaidFeature",
    "IsMaidPropensity",
    "GetYotogiSkillLevel",
    "IsYotogiSkill",
    "GetStockMaidCount",
    "SetSalonGrade",
    "GetSalonGrade",
    "GetClubEvaluation",
    "SetClubGauge",
    "GetClubGauge",
    "GetBGName",
    "IsItemParam",
    "IsItemSlot",
    "IsMaidClass",
    "GetMaidClassLevel",
    "IsJobMaid",
    "GetJobMaidLevel",
    "IsYotogiClass",
    "GetYotogiClassLevel",
    "GetNoonScuccessLevel",
    "IsMaid",
    "IsOldPersonal",
    "GetSalonLoan",
    "GetPhase",
    "GetDay",
    "CreateStockMaidLoopData",
    "CreateCompetitiveShowLoopData",
    "GetCompetitiveShowLoopCount",
    "SetActiveSlotFromStockMaidLoopData",
    "SetMaidCondition",
    "SetSeikeikenFront",
    "SetSeikeikenBack",
    "SetInitSeikeiken",
    "SetNewItem",
    "IsGameInShopItemPurchased",
    "SetContractType",
    "SetFeature",
    "SetPropensity",
    "SetSexualParam",
    "SetNewTrophy",
    "CheckTrophyContractType",
    "SetYotogiSelectStageName",
    "GetYotogiSelectStageName",
    "ClearCallStack",
    "IsCommunicationChara",
    "SetCommunicationChara",
    "SetRandomCommuChara",
    "CreateYotogiCharaList",
    "CreateVipCharaList",
    "CreateFacilityCharaList",
    "IsReceptionChara",
    "SetRandomReceptionChara",
    "IsSelectChara",
    "GetDateYear",
    "GetDateMonth",
    "GetDateDay",
    "UpdateMaidRanking",
    "ResetMaidRanking",
    "SetMaidRanking1st",
    "IsAvailableRanking",
    "IsMVPEvent",
    "IsYotogiSubCharaSelect",
    "GetYotogiSkillSpecialType",
    "IsSelectedYotogiSkill",
    "IsExistentFile",
    "IsExistentFileLegacy",
    "CheckClearVIP",
    "IsCBLVersion",
    "GetRentalNPCName",
    "IsMarriage",
    "IsNewWifeFlag",
    "IsPluginPack",
    "AddShopLineup",
    "SetEventItemFlag",
    "IsEventItemFlag",
    "GetVRMode",
    "IsVRMode",
    "SetDanceData",
    "GetEventAdded",
    "GetLockNTRPlayFlag",
    "GetLockUserDraftMaid",
    "IsComPlayer",
    "SetPlayerMoney",
    "GetPlayerMoney",
    "SetMaidLeader",
    "SetMaidCostumeFacility",
    "ResetMaidCostumeFacility",
    "CallTitleBar",
    "SetCompatibilityMode",
    "IsBuildingFacility",
    "IsBuildingFacilityUpward",
    "CheckDanceType",
    "SetMusicRelease",
    "GetChallengeRank",
    "BjDayFirst",
    "CasinoShopBuy",
    "SetEventEndFlag",
    "IsKasizukiGuest",
    "GetKasizukiSelectRoomName",
    "GetKasizukiMaidLikability",
    "GetKasizukiMaidLikabilityName",
    "GetKasizukiMaidWorkCount",
    "GetKasizukiMaidWorkCountName",
    "GetKasizukiSystemFlag",
    "SetKasizukiSystemFlag",
    "AddKasizukiSystemFlag",
    "SetKasizukiPassFlag",
    "GetKasizukiMaidFlag",
    "SetKasizukiMaidFlag",
    "IsKasizukiCorrectSenarioFlag",
    "IsVRKaraokeMode",
];

pub(super) const FUNCS: &[(&str, Func)] = &[
    ("GetLockNTRPlayFlag", |s, _| {
        Ok(boolean(s.player.flag("__lockNTRPlay") != 0))
    }),
    ("GetLockUserDraftMaid", |s, _| {
        Ok(boolean(s.player.flag("__lockUserDraftMaid") != 0))
    }),
    ("IsComPlayer", |s, _| {
        Ok(boolean(s.player.flag("__isComPlayer") == 0))
    }),
    // PlayerStatus.Status.isAvailableRanking is a literal false in this build.
    // Both IsMVPEvent and Maid.UpdateMaidRanking return at that same guard.
    ("IsAvailableRanking", |_, _| Ok(boolean(false))),
    ("IsMVPEvent", |_, _| Ok(boolean(false))),
    ("UpdateMaidRanking", |_, _| Ok(Variant::Void)),
    ("GetManFlag", |s, a| {
        ensure!(matches!(a.len(), 1 | 2), "expected flag name or slot and flag name");
        if a.len() == 2 { let _slot = arg_int(a, 0); }
        Ok(int(s.player.flag(&arg_string(a, a.len() - 1))))
    }),
    ("SetManFlag", |s, a| {
        ensure!(matches!(a.len(), 2 | 3), "expected [slot,] name, value");
        if s.retail.compatibility {
            return Ok(Variant::Void);
        }
        let offset = a.len() - 2;
        s.player
            .generic_flag
            .insert(arg_string(a, offset), arg_int(a, offset + 1) as i32);
        Ok(Variant::Void)
    }),
    ("SetPlayerMoney", |s, a| {
        ensure!(a.len() == 1, "expected amount");
        s.player.money = i64::from((arg_int(a, 0) as i32).max(0));
        Ok(Variant::Void)
    }),
    ("GetPlayerMoney", |s, _| {
        Ok(int(s.player.money.min(i32::MAX.into()) as i32))
    }),
    ("SetEventItemFlag", |s, a| {
        ensure!(a.len() == 2, "expected item, flag");
        let key = arg_string(a, 0).to_lowercase();
        let present = arg_int(a, 1) as i32 != 0;
        if !present {
            s.player.have_item.remove(&key);
        }
        s.player.have_item.insert(key, present);
        Ok(Variant::Void)
    }),
    ("IsEventItemFlag", |s, a| {
        ensure!(a.len() == 1, "expected item");
        Ok(boolean(
            s.player
                .have_item
                .get(&arg_string(a, 0).to_lowercase())
                .copied()
                .unwrap_or(false),
        ))
    }),
    ("SetClubGauge", |s, a| {
        ensure!(a.len() == 1, "expected gauge");
        s.club_gauge = (arg_int(a, 0) as i32).clamp(0, 100);
        Ok(Variant::Void)
    }),
    ("GetClubGauge", |s, _| Ok(int(s.club_gauge))),
    ("GetClubEvaluation", |s, _| {
        Ok(int(s.player.salon_evaluation))
    }),
    ("IsMaid", |s, a| {
        let [Variant::Integer(slot)] = a else {
            anyhow::bail!("expected integer maid slot")
        };
        Ok(boolean(
            s.active_maids
                .get(*slot as usize)
                .copied()
                .flatten()
                .is_some(),
        ))
    }),
    ("CheckClearVIP", |s, a| {
        ensure!(a.len() == 1, "expected work ID");
        Ok(boolean(
            s.player
                .night_works_state
                .get(&(arg_int(a, 0) as i32))
                .is_some_and(|w| w.finish),
        ))
    }),
    // The standalone assembly defines these three queries as literal returns.
    ("GetPhase", |_, _| Ok(int(2))),
    ("GetSalonLoan", |_, _| Ok(int(0))),
    ("IsCBLVersion", |_, _| Ok(boolean(false))),
    ("SetSystemFlag", |s, a| {
        ensure!(a.len() == 2, "expected name, value");
        let value = arg_int(a, 1) as i32;
        s.system_versions
            .insert(arg_string(a, 0), value.to_string());
        Ok(int(value))
    }),
    ("GetSystemFlag", |s, a| {
        ensure!(a.len() == 1, "expected name");
        let text = s
            .system_versions
            .get(&arg_string(a, 0))
            .map(String::as_str)
            .unwrap_or("");
        Ok(int(if text.is_empty() {
            0
        } else {
            text.trim().parse::<i32>()?
        }))
    }),
    ("SetSalonGrade", |s, a| {
        ensure!(a.len() == 1, "expected club grade");
        s.player.current_salon_grade = (arg_int(a, 0) as i32).clamp(1, 5);
        Ok(Variant::Void)
    }),
    ("AddShopLineup", |s, a| {
        ensure!(a.len() == 1, "expected item ID");
        let id = arg_int(a, 0) as i32;
        if !s.player.shop_lineup.contains(&id) {
            s.player.shop_lineup.insert(id, crate::params::SHOP_NEW);
        }
        Ok(Variant::Void)
    }),
    ("IsGameInShopItemPurchased", |s, a| {
        ensure!(a.len() == 1, "expected item ID");
        Ok(boolean(
            s.player.shop_lineup.get(&(arg_int(a, 0) as i32))
                == Some(&crate::params::SHOP_PURCHASED),
        ))
    }),
    // The standalone desktop host publishes a conventional screen (VRDeviceType.NON).
    ("GetVRMode", |_, _| Ok(int(0))),
    ("IsVRMode", |_, _| Ok(int(0))),
];

#[cfg(test)]
mod tests {
    use crate::params::NightWorkState;
    use crate::tjs::{ScriptProfile, Tjs, Variant};
    #[test]
    fn retail_ranking_gate_ignores_legacy_unlock_flag() -> anyhow::Result<()> {
        let mut vm = Tjs::for_profile(Default::default(), ScriptProfile::Com3d2)?;
        vm.exec("ranking", "SetManFlag('__ランキング解放', 1); SetManFlag('__ランキング日数', 7);")?;
        assert!(!vm.eval_bool("IsAvailableRanking()")?);
        assert!(!vm.eval_bool("IsMVPEvent()")?);
        assert!(matches!(vm.eval("UpdateMaidRanking()")?, Variant::Void));
        assert_eq!(vm.eval("GetManFlag(5, '__ランキング日数')")?, Variant::Integer(7));
        assert!(vm.eval("GetManFlag()").is_err());
        Ok(())
    }

    #[test]
    fn standalone_scripts_read_and_change_player_state() -> anyhow::Result<()> {
        let mut tjs = Tjs::for_profile(Default::default(), ScriptProfile::Com3d2)?;
        assert!(tjs.eval_bool("IsComPlayer()")?);
        assert!(matches!(
            tjs.eval("SetManFlag(0, '__isComPlayer', 1)")?,
            Variant::Void
        ));
        assert!(!tjs.eval_bool("IsComPlayer()")?);
        tjs.exec(
            "setup",
            "SetManFlag('__lockNTRPlay', 1); SetManFlag('__lockUserDraftMaid', 1);",
        )?;
        assert!(tjs.eval_bool("GetLockNTRPlayFlag() && GetLockUserDraftMaid()")?);
        tjs.exec(
            "items",
            "SetEventItemFlag('Ticket', 1); SetClubGauge(150); SetPlayerMoney(-3);",
        )?;
        assert!(tjs.eval_bool(
            "IsEventItemFlag('TICKET') && GetClubGauge() == 100 && GetPlayerMoney() == 0"
        )?);
        tjs.eval("SetEventItemFlag('ticket', 0)")?;
        assert!(!tjs.eval_bool("IsEventItemFlag('Ticket')")?);
        let s = &mut tjs.runtime.host_mut().state;
        s.player.money = 4_000_000_000;
        s.player.night_works_state.insert(
            42,
            NightWorkState {
                finish: true,
                ..Default::default()
            },
        );
        s.active_maids = vec![None, Some(3)];
        assert!(tjs.eval_bool(
            "GetPlayerMoney() == 2147483647 && CheckClearVIP(42) && !CheckClearVIP(1)"
        )?);
        assert!(tjs.eval_bool("IsMaid(1) && !IsMaid(0) && !IsVRMode() && GetVRMode() == 0")?);
        assert!(tjs.eval("SetClubGauge()").is_err());
        assert!(tjs.eval_bool("GetPhase() == 2 && GetSalonLoan() == 0 && !IsCBLVersion()")?);
        tjs.exec("system","SetManFlag('version', 1); SetSystemFlag('version', 23); SetSalonGrade(7); AddShopLineup(12);")?;
        assert!(tjs.eval_bool("GetSystemFlag('version') == 23 && GetManFlag('version') == 1 && GetSystemFlag('missing') == 0 && GetSalonGrade() == 5")?);
        assert!(!tjs.eval_bool("IsGameInShopItemPurchased(12)")?);
        tjs.runtime
            .host_mut()
            .state
            .player
            .shop_lineup
            .insert(12, crate::params::SHOP_PURCHASED);
        tjs.eval("AddShopLineup(12)")?;
        assert!(tjs.eval_bool("IsGameInShopItemPurchased(12)")?);
        let mut old = Tjs::new(Default::default())?;
        assert!(old.eval("IsComPlayer()").is_err());
        assert_eq!(old.eval_string("SetManFlag('legacy', 7)")?, "7");
        Ok(())
    }
}
