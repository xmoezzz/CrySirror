// SPDX-License-Identifier: MPL-2.0
use super::*;

/// Each native playing slot has a base skill and an optional character-owned
/// skill. Free and VR selections have a base record without owned skill data.
#[derive(Clone, Default)]
pub(super) struct Manager {
    pub legacy: bool,
    pub free: bool,
    pub script_call: bool,
    pub selected: Vec<Option<(i32, Option<i32>)>>,
}
impl State {
    pub(crate) fn scene_loaded(&mut self, name: &str, tag: &BTreeMap<String, String>) {
        self.vr_karaoke = name.contains("SceneVRCommunication").then(|| {
            tag.get("name")
                .is_some_and(|n| n.contains("SceneVRCommunication"))
                && tag.contains_key("karaoke")
        });
        self.selection = matches!(name, "SceneYotogi" | "SceneYotogiOld").then(|| Manager {
            legacy: name == "SceneYotogiOld",
            free: tag.contains_key("free_start"),
            script_call: tag.get("name").is_some_and(|n| n == name),
            selected: vec![None; 7],
        });
    }
}

pub(super) fn selected(s: &mut TjsState, a: &[Variant]) -> Result<Variant> {
    arity(a, 1)?;
    let Some(m) = s
        .retail
        .selection
        .as_ref()
        .filter(|m| !m.legacy && !m.selected.is_empty())
    else {
        return Ok(Variant::Void);
    };
    Ok(boolean(
        m.selected
            .iter()
            .flatten()
            .any(|(_, owned)| *owned == Some(arg_int(a, 0) as i32)),
    ))
}

pub(super) fn special(s: &mut TjsState, _: &[Variant]) -> Result<Variant> {
    let Some(m) = s
        .retail
        .selection
        .as_ref()
        .filter(|m| !m.legacy && !m.selected.is_empty())
    else {
        return Ok(Variant::Void);
    };
    let id = m.selected[0]
        .context("first playing slot has no base skill")?
        .0;
    let t = s.retail.table("yotogi_skill_list.nei")?;
    let r = row_id(&t, id)?;
    let cell = t.string(23, r);
    let n = if cell.is_empty() {
        0
    } else {
        cell.trim().parse::<i32>()?
    };
    let text = ["Null", "Drunk", "Mask", "Drug", "Faint", "Confess"]
        .get(n as usize)
        .map(|n| n.to_string())
        .unwrap_or_else(|| n.to_string());
    Ok(Variant::String(text))
}

pub(super) fn sub_select(s: &mut TjsState, _: &[Variant]) -> Result<Variant> {
    let old = s.retail.compatibility || s.player.flag("2.0モード") == 1;
    let Some(m) = s.retail.selection.as_ref().filter(|m| m.legacy == old) else {
        return Ok(boolean(false));
    };
    let ids: Vec<i32> = if m.free {
        let maid = s.maid(0).context("empty active maid for free selection")?;
        if old {
            s.retail
                .characters
                .get(&maid.guid_id)
                .map(|c| c.old_skill_ids.iter().copied().collect())
                .unwrap_or_default()
        } else {
            maid.skill_data.iter().map(|(id, _)| *id).collect()
        }
    } else {
        m.selected.iter().flatten().map(|(id, _)| *id).collect()
    };
    if ids.is_empty() {
        return Ok(boolean(false));
    }
    let possible = if m.script_call && !m.free {
        let t = s.retail.table("schedule_work_night.nei")?;
        let night = s.player.flag("時間帯") == 3;
        facility::roster(s)
            .iter()
            .filter(|i| {
                let maid = &s.maids[i];
                let id = if night {
                    maid.night_work_id
                } else {
                    maid.noon_work_id
                };
                row_id(&t, id).is_ok_and(|r| t.string(6, r) == "HaveSex")
            })
            .count()
    } else if old {
        s.stock
            .iter()
            .filter_map(|i| s.maids.get(i))
            .filter(|m| {
                !m.is_rental_maid
                    && s.retail
                        .characters
                        .get(&m.guid_id)
                        .is_some_and(|c| c.old.is_some())
            })
            .count()
    } else {
        s.stock.iter().filter(|i| s.maids.contains_key(i)).count()
    };
    let (t, col) = if old {
        let source = s.retail.source()?;
        let bytes = s
            .retail
            .legacy_files
            .read(source.storage.as_ref(), "yotogi_skil_detail.nei")?;
        let key = s
            .retail
            .key
            .as_ref()
            .context("missing linked legacy table key")?;
        (Arc::new(Table::open(&bytes, &key.table_key)?), 12)
    } else {
        (s.retail.table("yotogi_skill_list.nei")?, 13)
    };
    for id in ids {
        let n = t.int(col, row_id(&t, id)?).max(1);
        if n > 1 && n as usize <= possible {
            return Ok(boolean(true));
        }
    }
    Ok(boolean(false))
}
