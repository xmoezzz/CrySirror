// SPDX-License-Identifier: MPL-2.0
use super::*;
const RATE: i64 = 1_000;
const LIMIT: i64 = 999_999;
#[derive(Default)]
pub(super) struct CasinoMenu {
    venues: Vec<(usize, String)>,
    pub(super) exchange: Option<i64>,
    dealer_list: bool,
    dealers: Vec<usize>,
    pending_dealer: Option<usize>,
    dealer_controls: Vec<Widget>,
}
impl Game {
    pub(super) fn open_casino(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<String> {
        self.adv_camera = ([0., 1.45, -0.05], 0.1, [-180., 10.]);
        self.sound
            .play(host, c, &self.files, Kind::Bgm, "BGM010.ogg", 0., true)?;
        self.leisure.casino.dealer_controls = std::mem::take(&mut self.leisure.base);
        self.leisure.base = self
            .assets
            .filtered_canvas(op, host, c, Casino.name(), "Canvas", |p| {
                p.is_empty()
                    || ["/CasinoMenu", "/ExChangeUI", "/TopFlame"]
                        .iter()
                        .any(|v| p.starts_with(v))
            })?
            .base;
        let (venues, previous) = {
            let mut vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let s = &mut vm.runtime.host_mut().state;
            (activities::casinos(s)?, s.retail.casino_slot)
        };
        ensure!(!venues.is_empty(), "SceneCasino requires a built casino");
        self.leisure.casino.venues = venues;
        if self.leisure.casino.venues.len() == 2
            && !self.script.tag_backup.contains_key("return_casinomenu")
        {
            self.leisure_prompt(
                op,
                host,
                c,
                now,
                "どちらのカジノでプレイしますか？",
                "yesno",
                Response::Venue,
            )?;
            let names = &self.leisure.casino.venues;
            for w in &mut self.widgets {
                if let Some(label) = &mut w.label {
                    if w.path.contains("/Ok/") {
                        label.set_text(&names[0].1);
                    } else if w.path.contains("/Cancel/") {
                        label.set_text(&names[1].1);
                    }
                }
            }
        } else {
            let index = self
                .leisure
                .casino
                .venues
                .iter()
                .position(|(s, _)| Some(*s) == previous)
                .unwrap_or(0);
            self.casino_venue(index, op, host, c, now)?;
        }
        Ok("CasinoMini".into())
    }
    pub(super) fn casino_venue(
        &mut self,
        index: usize,
        op: &mut Opener,
        _host: &mut dyn HostServices,
        _c: ClientId,
        now: u64,
    ) -> Result<()> {
        let slot = self.leisure.casino.venues[index].0;
        let id = {
            let mut vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let s = &mut vm.runtime.host_mut().state;
            s.retail.casino_slot = Some(slot);
            let id = match s.retail.casino_dealers.get(&slot) {
                Some(&id) if s.stock.contains(&id) => id,
                _ => activities::first_dealer(s)?,
            };
            s.retail.casino_dealers.insert(slot, id);
            self.leisure.casino.dealers = activities::entrants(s);
            id
        };
        self.leisure_actor(id, op, now)?;
        self.actors.apply(
            crate::kag::AdvUi::AllOffset {
                position: Some([0., 0., -2.]),
                rotation: Some([0.; 3]),
            },
            &self.script,
            &self.files,
            &mut self.assets,
            op,
            now,
        )?;
        self.casino_widgets()
    }
    pub(super) fn casino_widgets(&mut self) -> Result<()> {
        let vm = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let s = &vm.runtime.host().state;
        let menu = &self.leisure.casino;
        let total = s.retail.casino_coins;
        let money = s.player.money;
        self.widgets = Vec::new();
        let mut digit_count = [0; 3];
        for base in &self.leisure.base {
            let mut w = base.clone();
            let path = w.path.as_str();
            if menu.dealer_list {
                continue;
            }
            let is_exchange = path.starts_with("/ExChangeUI");
            if is_exchange != menu.exchange.is_some() && !path.starts_with("/TopFlame") {
                continue;
            }
            let name = path.rsplit('/').next().unwrap_or("");
            if path
                .strip_prefix("/CasinoMenu/")
                .is_some_and(|p| !p.contains('/'))
                || path == "/TopFlame/ButtonKankin" && menu.exchange.is_none()
                || is_exchange && matches!(name, "Up" | "Down" | "OK" | "Cancel" | "Reset" | "Max")
            {
                w.collider = Some(w.rect);
            }
            let mut text = None;
            if let Some(v) = menu.exchange {
                if let Some(kind) = ["Up", "Down", "Number"].iter().position(|n| *n == name) {
                    let digit = 5 - digit_count[kind];
                    digit_count[kind] += 1;
                    let scale = 10i64.pow(digit);
                    w.path = format!("exchange/{name}/{scale}");
                    if kind == 2 {
                        text = Some((v / scale % 10).to_string());
                    } else if Self::exchange_max(money, total) < scale {
                        w.collider = None;
                    }
                } else {
                    text = match path {
                        "/ExChangeUI/RateText" => {
                            Some(format!("コイン1枚 = {}CR", daily::money(RATE)))
                        }
                        "/ExChangeUI/Balance" => Some(format!("-{}", daily::money(v * RATE))),
                        "/ExChangeUI/Current/Text" => Some(daily::money(total + v)),
                        _ => None,
                    };
                }
            }
            if w.path == "/TopFlame/Coin" {
                text = Some(daily::money(total));
            }
            if w.path == "/TopFlame/Sikin" {
                text = Some(daily::money(money));
            }
            if let (Some(l), Some(v)) = (&mut w.label, text) {
                l.set_text(&v);
            }
            self.widgets.push(w);
        }
        if menu.dealer_list {
            self.widgets.extend(menu.dealer_controls.clone());
            let template = self
                .leisure
                .base
                .iter()
                .find(|w| w.path == "/CasinoMenu/SelectDealer/Text")
                .context("dealer button label")?;
            for (i, id) in menu.dealers.iter().enumerate() {
                let m = &s.maids[id];
                let mut w = template.clone();
                w.path = format!("dealer/{id}");
                w.rect = [40., 60. + i as f32 * 54., 340., 48.];
                if menu.pending_dealer == Some(*id) {
                    w.tint = [255, 215, 130, 255];
                }
                w.collider = Some(w.rect);
                w.label
                    .as_mut()
                    .context("dealer label")?
                    .set_text(&format!("{} {}", m.last_name, m.first_name));
                self.widgets.push(w);
            }
        }
        Ok(())
    }
    fn exchange_max(money: i64, coins: i64) -> i64 {
        (LIMIT - coins).min(money / RATE)
    }
    pub(super) fn casino_click(
        &mut self,
        _i: usize,
        path: &str,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<()> {
        let (money, coins) = {
            let vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let s = &vm.runtime.host().state;
            (s.player.money, s.retail.casino_coins)
        };
        let max = Self::exchange_max(money, coins);
        if let Some(id) = path.strip_prefix("dealer/") {
            self.leisure.casino.pending_dealer = Some(id.parse()?);
        } else if path.ends_with("/Ok_Button") {
            let id = self
                .leisure
                .casino
                .pending_dealer
                .context("missing dealer selection")?;
            {
                let mut vm = self
                    .script
                    .tjs
                    .lock()
                    .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                let s = &mut vm.runtime.host_mut().state;
                s.retail
                    .casino_dealers
                    .insert(s.retail.casino_slot.context("dealer venue")?, id);
            }
            self.leisure_actor(id, op, now)?;
            self.leisure.casino.dealer_list = false;
        } else if let Some(value) = self.leisure.casino.exchange {
            if let Some(digit) = path.strip_prefix("exchange/") {
                let (dir, pow) = digit.split_once('/').context("coin digit")?;
                let pow = pow.parse::<i64>()?;
                let digit = value / pow % 10;
                let delta = match dir {
                    "Up" if value == max => -(max / pow).min(9),
                    "Up" if digit == 9 => -9,
                    "Up" => 1,
                    "Down" if value == 1 => 9 - (1 / pow),
                    "Down" if digit == 0 => 9,
                    "Down" => -1,
                    _ => return Ok(()),
                };
                self.leisure.casino.exchange = Some((value + delta * pow).clamp(1, max));
            } else {
                match path {
                    "/ExChangeUI/Max" => self.leisure.casino.exchange = Some(max),
                    "/ExChangeUI/Reset" => self.leisure.casino.exchange = Some(1),
                    "/ExChangeUI/Cancel" => self.leisure.casino.exchange = None,
                    "/ExChangeUI/OK" => {
                        ensure!((1..=max).contains(&value), "invalid coin exchange");
                        let mut vm = self
                            .script
                            .tjs
                            .lock()
                            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                        let s = &mut vm.runtime.host_mut().state;
                        s.player.money -= value * RATE;
                        s.retail.casino_coins += value;
                        self.leisure.casino.exchange = None;
                    }
                    _ => {}
                }
            }
        } else {
            match path {
                "/CasinoMenu/Cancel_Button" if self.leisure.casino.dealer_list => {
                    self.leisure.casino.dealer_list = false
                }
                "/CasinoMenu/Cancel_Button" => {
                    return self.resume_at("cancel_label", op, host, c, now);
                }
                "/CasinoMenu/SelectDealer" => {
                    self.leisure.casino.dealer_list = true;
                    let vm = self
                        .script
                        .tjs
                        .lock()
                        .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                    let s = &vm.runtime.host().state;
                    self.leisure.casino.pending_dealer = s
                        .retail
                        .casino_slot
                        .and_then(|slot| s.retail.casino_dealers.get(&slot).copied());
                }
                "/TopFlame/ButtonKankin" if money < RATE || coins >= LIMIT - 1 => {
                    return self.leisure_prompt(
                        op,
                        host,
                        c,
                        now,
                        if money < RATE {
                            "換金するには最低でも\n資金が1,000CR必用です"
                        } else {
                            "これ以上換金できません"
                        },
                        "ok",
                        Response::Dismiss,
                    );
                }
                "/TopFlame/ButtonKankin" => self.leisure.casino.exchange = Some(1),
                "/CasinoMenu/BlackJack" if coins < 100 => {
                    return self.leisure_prompt(
                        op,
                        host,
                        c,
                        now,
                        "ブラックジャックをするには最低でも\nコインが100枚必用です",
                        "ok",
                        Response::Dismiss,
                    );
                }
                "/CasinoMenu/CasinoSlot" if coins < 300 => {
                    return self.leisure_prompt(
                        op,
                        host,
                        c,
                        now,
                        "スロットをするには最低でも\nコインが300枚必用です",
                        "ok",
                        Response::Dismiss,
                    );
                }
                p if let Some(button) = p.strip_prefix("/CasinoMenu/") => {
                    let key = match button {
                        "BlackJack" => "Blackjack",
                        "CasinoSlot" => "Slot",
                        key => key,
                    };
                    {
                        let mut vm = self
                            .script
                            .tjs
                            .lock()
                            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                        let s = &mut vm.runtime.host_mut().state;
                        let t = s.retail.table("casinomenu_data.nei")?;
                        let row = (1..t.rows)
                            .find(|&r| t.string(0, r) == key)
                            .context("casino menu label")?;
                        if key == "SetCostume" {
                            s.retail.edit_facility = s.retail.casino_slot;
                        }
                        vm.eval(&format!(
                            "tf['menu_label_name'] = {}",
                            serde_json::to_string(&t.string(1, row))?
                        ))?;
                    }
                    return self.resume_script(op, host, c, now);
                }
                _ => {}
            }
        }
        self.casino_widgets()
    }
}
