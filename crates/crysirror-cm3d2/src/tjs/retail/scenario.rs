// SPDX-License-Identifier: MPL-2.0
//! ScenarioSelectMgr's important-event notifications. Candidate maids are
//! intersected between columns, with the native per-token add/remove order.
use super::*;
use crate::params::MaidStatus;

#[derive(Clone, Default)]
pub(super) struct Events {
    previous: BTreeSet<i32>,
    played: BTreeSet<i32>,
    fixed: BTreeSet<i32>,
    pub added: Vec<i32>,
    catalog: Option<Arc<Catalog>>,
}
struct Catalog {
    table: Arc<Table>,
    names: BTreeMap<String, i32>,
    ids: BTreeSet<i32>,
}
impl Catalog {
    fn load(s: &mut State) -> Result<Arc<Self>> {
        if let Some(catalog) = &s.events.catalog {
            return Ok(catalog.clone());
        }
        let table = s.table("select_scenario_data.nei")?;
        let mut ids = BTreeSet::new();
        let mut names = BTreeMap::new();
        for r in (1..table.rows).filter(|&r| table.exists(0, r)) {
            let id = table.int(0, r);
            if s.enabled("selectable_scenario_id", id)? && ids.insert(id) {
                names
                    .entry(table.string(1, r).replace('\n', ""))
                    .or_insert(id);
            }
        }
        let catalog = Arc::new(Self { table, names, ids });
        s.events.catalog = Some(catalog.clone());
        Ok(catalog)
    }
}

fn personality(s: &mut State, m: &MaidStatus) -> Result<String> {
    let t = s.table("maid_status_personal_list.nei")?;
    let personal = t.string(1, row_id(&t, m.personal)?);
    let t = s.table("personal_convert_data.nei")?;
    Ok((2..t.rows)
        .find(|&r| t.string(0, r) == personal)
        .map(|r| t.string(1, r))
        .unwrap_or_default())
}

fn cmp_param(m: &MaidStatus, x: &character::Status, token: &str) -> Result<bool> {
    let token = token
        .replace([' ', '　'], "")
        .replace('＋', "+")
        .replace('ー', "-")
        .replace('＝', "=");
    let sep = if token.contains('=') {
        '='
    } else if token.contains('-') {
        '-'
    } else {
        '+'
    };
    let (name, n) = token
        .split_once(sep)
        .context("malformed scenario parameter")?;
    let mut n = n.parse::<i32>()?;
    if sep == '-' {
        n = n.wrapping_neg();
    }
    let v = match name {
        "興奮" => m.a_cur_excite(),
        "官能" => x.sensual,
        "理性" => m.a_reason(),
        "体力" => m.a_hp(),
        "精神" => m.a_mind(),
        "AP" => character::parameter(m, x, "アピールポイント")?,
        _ => character::parameter(m, x, name)?,
    };
    Ok(v >= n)
}

fn owns_class(t: &Table, classes: &BTreeMap<i32, i32>, name: &str) -> Result<bool> {
    // This build indexes the sorted dictionary by integer key, not by ordinal.
    for i in 0..classes.len() as i32 {
        ensure!(
            classes.contains_key(&i),
            "scenario class dictionary has no key {i}"
        );
        if t.string(2, row_id(t, i)?) == name {
            return Ok(true);
        }
    }
    Ok(false)
}

struct Candidates {
    maids: BTreeSet<usize>,
    fixed: bool,
    touched: bool,
}
impl Candidates {
    fn column(
        &mut self,
        s: &mut TjsState,
        c: &Catalog,
        id: i32,
        col: usize,
        value: &str,
    ) -> Result<bool> {
        let expr = value.replace('．', ".").replace(['，', ','], "&");
        let and = expr.contains('&');
        let or = expr.contains('|');
        let sep = match (and, or) {
            (true, false) => '&',
            (false, true) => '|',
            _ => ',',
        };
        let mut ok = and;
        let mut update = |b| if and { ok &= b } else { ok |= b };
        let mut first = true;
        for token in expr.split(sep) {
            if col == 12 {
                let t = s.retail.table("facility_material.nei")?;
                let mid = (1..t.rows)
                    .find(|&r| t.string(1, r) == token)
                    .map(|r| t.int(0, r));
                update(mid.is_some_and(|v| s.retail.materials.contains(&v)));
                continue;
            }
            if col == 13 {
                let t = s.retail.table("facility.nei")?;
                let tok = token.replace("Lv", " ");
                let mut tok = tok.split(' ');
                let name = tok.next().unwrap_or("");
                let fid = name.parse::<i32>().ok().or_else(|| {
                    (1..t.rows)
                        .find(|&r| t.string(2, r) == name)
                        .map(|r| t.int(0, r))
                });
                let level = tok.next().map(str::parse::<i32>).transpose()?;
                let owns = if let Some(fid) = fid {
                    let has = facility::built(s, &t, fid);
                    has && level.is_none_or(|v| {
                        s.retail.facility_levels.get(&fid).copied().unwrap_or(1) >= v
                    })
                } else {
                    false
                };
                update(owns);
                continue;
            }
            let reference = if col == 5 {
                token
                    .parse::<i32>()
                    .ok()
                    .filter(|n| c.ids.contains(n))
                    .or_else(|| c.names.get(token).copied())
            } else {
                None
            };
            if col == 5 && reference.is_none() {
                if token.contains("シナリオフラグ") {
                    let flag = token.split(':').nth(1).context("missing scenario flag")?;
                    update(s.player.flag(flag) == 1);
                    continue;
                }
                if !token.contains("メイドフラグ") {
                    continue;
                }
            }
            if first {
                self.maids.clear();
                first = false;
                self.touched = true;
            }
            for &idx in &s.stock {
                let m = s.maids.get(&idx).context("missing stock maid")?;
                let x = s
                    .retail
                    .characters
                    .get(&m.guid_id)
                    .cloned()
                    .unwrap_or_default();
                let yes = match col {
                    3 => m.likability >= token.parse::<i32>()?,
                    5 => {
                        if let Some(prev) = reference {
                            s.retail.events.played.contains(&prev)
                                && s.retail
                                    .event_end
                                    .get(&m.guid_id)
                                    .and_then(|v| v.get(&prev))
                                    .copied()
                                    .unwrap_or(false)
                        } else {
                            m.flag(token.split(':').nth(1).context("missing maid flag")?) == 1
                        }
                    }
                    6 => {
                        let main = s.retail.main_characters.contains(&idx);
                        if token.contains("NPCキャラ") {
                            self.fixed = true;
                            if x.sub_name.as_deref() != token.split('.').nth(1)
                                || x.sub_name.is_none()
                            {
                                continue;
                            }
                            true
                        } else if token.contains("NPC以外") {
                            x.sub_name.is_none()
                        } else {
                            let transferred = token.contains("移籍");
                            if x.sub_name.is_some() || x.old.is_some() != transferred {
                                continue;
                            }
                            match token {
                                "移籍メイド" | "オリジナルメイド" => true,
                                "雇用移籍メイド" | "雇用メイド" => !main,
                                _ => {
                                    let personal = personality(&mut s.retail, m)?;
                                    if token.contains("固定") {
                                        self.fixed = true;
                                        main && (matches!(token, "固定移籍メイド" | "固定メイド")
                                            || token
                                                == format!(
                                                    "固定{}{personal}",
                                                    if transferred { "移籍" } else { "" }
                                                ))
                                    } else {
                                        if personal.is_empty() {
                                            continue;
                                        }
                                        token
                                            == format!(
                                                "{}{personal}",
                                                if transferred { "移籍" } else { "" }
                                            )
                                    }
                                }
                            }
                        }
                    }
                    7 => cmp_param(m, &x, token)?,
                    8 => {
                        let t = s.retail.table("maid_status_propensity_list.nei")?;
                        (1..t.rows).any(|r| t.string(2, r) == token && m.is_propensity(t.int(0, r)))
                    }
                    9 | 10 => {
                        let job = col == 9;
                        let t = s.retail.table(if job {
                            "maid_status_jobclass_list.nei"
                        } else {
                            "maid_status_yotogiclass_list.nei"
                        })?;
                        owns_class(
                            &t,
                            if job {
                                &x.job_levels
                            } else {
                                &x.skill_class_levels
                            },
                            token,
                        )?
                    }
                    11 => ["お近づき", "信頼", "恋人"].get(x.relation as usize) == Some(&token),
                    _ => bail!("unknown scenario condition column {col}"),
                };
                if yes {
                    if !s
                        .retail
                        .event_end
                        .get(&m.guid_id)
                        .and_then(|flags| flags.get(&id))
                        .copied()
                        .unwrap_or(false)
                    {
                        self.maids.insert(idx);
                    }
                    update(true);
                } else if self.maids.remove(&idx) && and {
                    update(false);
                }
            }
        }
        Ok(ok)
    }
}

pub(super) fn added(s: &mut TjsState, _: &[Variant]) -> Result<Variant> {
    let c = Catalog::load(&mut s.retail)?;
    let mut now = BTreeSet::new();
    let mut added = Vec::new();
    for r in (1..c.table.rows).filter(|&r| c.table.string(18, r) == "重要") {
        let id = c.table.int(0, r);
        if !c.ids.contains(&id) || !c.table.exists(19, r) && s.retail.events.played.contains(&id) {
            continue;
        }
        let mut candidates = Candidates {
            maids: BTreeSet::new(),
            fixed: s.retail.events.fixed.contains(&id),
            touched: false,
        };
        let mut had_maids = false;
        let mut playable = false;
        for col in (3..=13).filter(|&col| col != 4 && c.table.exists(col, r)) {
            let before = candidates.maids.clone();
            playable = candidates.column(s, &c, id, col, &c.table.string(col, r))?;
            if !playable {
                break;
            }
            if had_maids && candidates.touched {
                candidates.maids.retain(|i| before.contains(i));
            }
            if candidates.touched && candidates.maids.is_empty() {
                playable = false;
                break;
            }
            had_maids |= !candidates.maids.is_empty();
        }
        if candidates.fixed {
            s.retail.events.fixed.insert(id);
        }
        let enough = if candidates.fixed {
            !candidates.maids.is_empty()
        } else {
            c.table.int(17, r) <= candidates.maids.len() as i32
        };
        if playable && enough && now.insert(id) && !s.retail.events.previous.contains(&id) {
            added.push(id);
        }
    }
    s.retail.events.previous = now;
    s.retail.events.added = added;
    Ok(boolean(!s.retail.events.added.is_empty()))
}
