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

pub(super) fn restore_slot(state: &mut State, saved: &crate::save::management_1040::Scenario) -> Result<()> {
    let catalog = Catalog::load(state)?;
    state.events.played = saved.played.iter()
        .filter_map(|&(id, played)| (played && catalog.ids.contains(&id)).then_some(id))
        .collect();
    state.events.previous = saved.selectable.iter().copied().filter(|id| catalog.ids.contains(id)).collect();
    state.events.fixed.clear();
    state.events.added.clear();
    Ok(())
}

pub(super) fn capture_slot(state: &mut State) -> Result<crate::save::management_1040::Scenario> {
    let catalog = Catalog::load(state)?;
    let mut seen = BTreeSet::new();
    let played = (1..catalog.table.rows)
        .filter(|&r| catalog.table.exists(0, r))
        .map(|r| catalog.table.int(0, r))
        .filter(|id| catalog.ids.contains(id) && seen.insert(*id))
        .map(|id| (id, state.events.played.contains(&id)))
        .collect();
    Ok(crate::save::management_1040::Scenario {
        played,
        selectable: state.events.previous.iter().copied().collect(),
    })
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
            if col == 4 {
                update((token == "昼") == (s.player.flag("時間帯") != 3));
                continue;
            }
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
                        s.retail.facility_exp.get(&fid).map_or(1, |xp| xp.level) >= v
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

fn evaluate(
    s: &mut TjsState,
    c: &Catalog,
    r: usize,
    important: bool,
) -> Result<Option<Candidates>> {
    let id = c.table.int(0, r);
    let mut candidates = Candidates {
        maids: BTreeSet::new(),
        fixed: s.retail.events.fixed.contains(&id),
        touched: false,
    };
    let mut had_maids = false;
    let mut playable = !important && !(3..=13).any(|col| c.table.exists(col, r));
    for col in (3..=13).filter(|&col| (!important || col != 4) && c.table.exists(col, r)) {
        let before = candidates.maids.clone();
        playable = candidates.column(s, c, id, col, &c.table.string(col, r))?;
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
            || !(3..=13).any(|col| c.table.exists(col, r))
    };
    Ok((playable && enough).then_some(candidates))
}

#[derive(Debug)]
pub(crate) struct Selection {
    pub id: i32,
    pub title: String,
    pub contents: String,
    pub icon: String,
    pub conditions: Vec<String>,
    pub maids: Vec<usize>,
    pub fixed: bool,
    script: String,
    label: String,
}
pub(crate) fn selections(s: &mut TjsState) -> Result<Vec<Selection>> {
    let c = Catalog::load(&mut s.retail)?;
    let mut result = Vec::new();
    for r in (1..c.table.rows).filter(|&r| c.table.exists(0, r)) {
        let id = c.table.int(0, r);
        if !c.ids.contains(&id) || (!c.table.exists(19, r) && s.retail.events.played.contains(&id))
        {
            continue;
        }
        let Some(candidates) = evaluate(s, &c, r, false)? else {
            continue;
        };
        let label = c.table.string(16, r);
        let mut maids: Vec<_> = s
            .stock
            .iter()
            .copied()
            .filter(|i| candidates.maids.contains(i))
            .collect();
        if !candidates.fixed {
            s.sort_maid_standard(&mut maids);
        }
        result.push(Selection {
            id,
            title: c.table.string(1, r),
            contents: c.table.string(2, r),
            icon: c.table.string(14, r),
            conditions: c
                .table
                .string(21, r)
                .split(['\n', '&'])
                .take(12)
                .map(str::to_owned)
                .collect(),
            maids,
            fixed: candidates.fixed,
            script: c.table.string(15, r),
            label: if !label.is_empty() && !label.contains('*') {
                format!("*{label}")
            } else {
                label
            },
        });
    }
    Ok(result)
}
impl Selection {
    pub(crate) fn play(&self, vm: &mut crate::tjs::Tjs, choice: Option<usize>) -> Result<()> {
        let s = &mut vm.runtime.host_mut().state;
        let selected = if self.fixed {
            self.maids.clone()
        } else {
            choice.into_iter().collect()
        };
        ensure!(
            selected
                .iter()
                .all(|i| self.maids.contains(i) && s.maids.contains_key(i)),
            "invalid scenario character"
        );
        ensure!(
            !self.script.is_empty() && !self.label.is_empty(),
            "missing scenario script or label"
        );
        let mut file = self.script.clone();
        if let Some(&id) = selected.first()
            && file.contains('?')
        {
            let m = &s.maids[&id];
            let marker = ["?4", "?3", "?2", "?1", "?0", "?"]
                .into_iter()
                .find(|p| file.contains(p))
                .unwrap();
            ensure!(
                matches!(marker, "?" | "?0"),
                "scenario placeholder {marker} needs another character"
            );
            let replacement = if let Some(name) = s
                .retail
                .characters
                .get(&m.guid_id)
                .and_then(|x| x.sub_name.clone())
            {
                let t = s.retail.table("maid_status_submaid_list.nei")?;
                t.string(2, stock::named(&t, 1, &name)?)
            } else {
                let t = s.retail.table("maid_status_personal_list.nei")?;
                t.string(3, row_id(&t, m.personal)?)
            };
            file = file.replace(marker, &replacement);
        }
        for (slot, &id) in selected.iter().enumerate() {
            if !self.fixed {
                facility::activate(s, slot, id, false);
            }
            s.retail
                .event_end
                .entry(s.maids[&id].guid_id.clone())
                .or_default()
                .insert(self.id, true);
        }
        s.retail.events.played.insert(self.id);
        vm.eval(&format!(
            "tf['scenario_file_name']={}",
            serde_json::to_string(&file)?
        ))?;
        vm.eval(&format!(
            "tf['label_name']={}",
            serde_json::to_string(&self.label)?
        ))?;
        Ok(())
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
        if evaluate(s, &c, r, true)?.is_some()
            && now.insert(id)
            && !s.retail.events.previous.contains(&id)
        {
            added.push(id);
        }
    }
    s.retail.events.previous = now;
    s.retail.events.added = added;
    Ok(boolean(!s.retail.events.added.is_empty()))
}
