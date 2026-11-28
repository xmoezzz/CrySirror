// SPDX-License-Identifier: MPL-2.0
//! Rules from standalone PlayerHand, Dealer, BjPlayer and CardDeck.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Card(u8);

impl Card {
    fn rank(self) -> u8 {
        self.0 % 13 + 1
    }

    fn display(self) -> String {
        let rank = [
            "A", "2", "3", "4", "5", "6", "7", "8", "9", "10", "J", "Q", "K",
        ][(self.rank() - 1) as usize];
        let suit = ["♠", "♦", "♥", "♣"][(self.0 / 13) as usize];
        format!("{rank}{suit}")
    }
}

#[derive(Clone, Default)]
pub(super) struct Hand {
    pub cards: Vec<Card>,
    pub bet: i64,
    stood: bool,
    surrendered: bool,
}

fn score(cards: &[Card]) -> u8 {
    let mut points = cards
        .iter()
        .filter(|c| c.rank() != 1)
        .map(|c| c.rank().min(10))
        .sum::<u8>();
    for _ in cards.iter().filter(|c| c.rank() == 1) {
        points += if points + 11 <= 21 { 11 } else { 1 };
    }
    points
}

impl Hand {
    pub fn score(&self) -> u8 {
        score(&self.cards)
    }

    fn ended(&self) -> bool {
        self.stood || self.surrendered || self.cards.len() >= 5 || self.score() >= 21
    }

    fn natural(&self, split: bool) -> bool {
        self.cards.len() == 2 && self.score() == 21 && !split
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Hit,
    Stand,
    Surrender,
    Double,
    Split,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Win,
    Natural,
    Draw,
    Loss,
    Surrender,
}

#[derive(Default)]
pub(super) struct Round {
    pub hands: Vec<Hand>,
    pub dealer: Vec<Card>,
    pub turn: usize,
    pub result: Vec<Outcome>,
    pub change: i64,
    deck: Vec<Card>,
}

impl Round {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn start(
        &mut self,
        bet: i64,
        coins: &mut i64,
        mut random: impl FnMut(usize) -> usize,
    ) -> Result<()> {
        ensure!((100..=*coins).contains(&bet), "unaffordable blackjack bet");
        self.clear();
        self.deck = (0..52).map(Card).collect();
        for i in 0..self.deck.len() {
            let other = i + random(self.deck.len() - i);
            self.deck.swap(i, other);
        }
        self.hands.push(Hand {
            bet,
            ..Default::default()
        });
        *coins -= bet;
        for _ in 0..2 {
            self.deal_player(0)?;
            self.deal_dealer()?;
        }
        self.advance_turn(coins)
    }

    pub fn permits(&self, action: Action, coins: i64) -> bool {
        let Some(hand) = self.hands.get(self.turn) else {
            return false;
        };
        if hand.ended() || !self.result.is_empty() {
            return false;
        }
        match action {
            Action::Hit | Action::Stand => true,
            Action::Surrender => self.hands.len() == 1 && hand.cards.len() == 2,
            // BjPlayer charges the standard bet when doubling either hand.
            Action::Double => hand.cards.len() == 2 && coins >= self.hands[0].bet,
            Action::Split => {
                self.hands.len() == 1
                    && hand.cards.len() == 2
                    && hand.cards[0].rank() == hand.cards[1].rank()
                    && coins >= hand.bet
            }
        }
    }

    pub fn play(&mut self, action: Action, coins: &mut i64) -> Result<()> {
        ensure!(self.permits(action, *coins), "invalid blackjack action");
        match action {
            Action::Hit => self.deal_player(self.turn)?,
            Action::Stand => self.hands[self.turn].stood = true,
            Action::Surrender => self.hands[self.turn].surrendered = true,
            Action::Double => {
                let extra = self.hands[0].bet;
                *coins -= extra;
                self.hands[self.turn].bet += extra;
                self.deal_player(self.turn)?;
                self.hands[self.turn].stood = true;
            }
            Action::Split => {
                let bet = self.hands[0].bet;
                let card = self.hands[0].cards.remove(1);
                self.hands.push(Hand {
                    cards: vec![card],
                    bet,
                    ..Default::default()
                });
                *coins -= bet;
                self.turn = 1;
                self.deal_player(1)?;
            }
        }
        self.advance_turn(coins)
    }

    fn deal_player(&mut self, index: usize) -> Result<()> {
        self.hands[index]
            .cards
            .push(self.deck.pop().context("blackjack deck exhausted")?);
        Ok(())
    }

    fn deal_dealer(&mut self) -> Result<()> {
        self.dealer
            .push(self.deck.pop().context("blackjack deck exhausted")?);
        Ok(())
    }

    fn advance_turn(&mut self, coins: &mut i64) -> Result<()> {
        if self.hands[self.turn].ended() && self.turn == 1 {
            self.turn = 0;
            self.deal_player(0)?;
        }
        if !self.hands.iter().all(Hand::ended) {
            return Ok(());
        }
        while score(&self.dealer) <= 16 && self.dealer.len() < 7 {
            self.deal_dealer()?;
        }
        let dealer_score = score(&self.dealer);
        let dealer_natural = self.dealer.len() == 2 && dealer_score == 21;
        let split = self.hands.len() == 2;
        let mut returned = 0;
        let mut staked = 0;
        for hand in &self.hands {
            let points = hand.score();
            let natural = hand.natural(split);
            let (outcome, payout) = if points > 21 {
                (Outcome::Loss, 0)
            } else if hand.surrendered {
                (Outcome::Surrender, hand.bet / 2)
            } else if natural && !dealer_natural {
                (Outcome::Natural, hand.bet * 5 / 2)
            } else if points == dealer_score && natural == dealer_natural {
                (Outcome::Draw, hand.bet)
            } else if points > dealer_score || dealer_score > 21 {
                (Outcome::Win, hand.bet * 2)
            } else {
                (Outcome::Loss, 0)
            };
            returned += payout;
            staked += hand.bet;
            self.result.push(outcome);
        }
        self.change = returned - staked;
        *coins = (*coins + returned).min(999_999);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_blackjack_natural_and_split_settlement() -> Result<()> {
        let mut round = Round::default();
        let mut coins = 900;
        round.hands.push(Hand {
            cards: vec![Card(0), Card(12)],
            bet: 100,
            ..Default::default()
        });
        round.dealer = vec![Card(18), Card(23)];
        round.deck.push(Card(38));
        round.advance_turn(&mut coins)?;
        assert_eq!(coins, 1_150);
        assert_eq!(round.result, [Outcome::Natural]);

        let mut coins = 900;
        round = Round {
            hands: vec![Hand {
                cards: vec![Card(7), Card(20)],
                bet: 100,
                ..Default::default()
            }],
            dealer: vec![Card(11), Card(19)],
            deck: vec![Card(4), Card(8), Card(9)],
            ..Default::default()
        };
        round.play(Action::Split, &mut coins)?;
        assert_eq!((round.turn, coins), (1, 800));
        round.play(Action::Stand, &mut coins)?;
        assert_eq!(round.turn, 0);
        round.play(Action::Double, &mut coins)?;
        assert_eq!(round.result, [Outcome::Loss, Outcome::Win]);
        assert_eq!(coins, 900);
        Ok(())
    }
}

const MIN: i64 = 100;
const MAX: i64 = 50_000;
const NORMAL: ([f32; 3], f32, [f32; 2]) = ([5., 0.88, 0.], 1.9, [90., 30.]);
const ZOOM: ([f32; 3], f32, [f32; 2]) = ([4.85, 0.88, 0.], 1.2, [90., 90.]);

#[derive(Default)]
pub(super) struct Table {
    panel: Vec<Widget>,
    round: Round,
    bet: Option<i64>,
    zoom: bool,
}

impl Game {
    pub(super) fn open_blackjack(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<String> {
        let dealer = {
            let vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let s = &vm.runtime.host().state.retail;
            *s.casino_dealers
                .get(&s.casino_slot.context("blackjack venue")?)
                .context("blackjack dealer")?
        };
        self.leisure_activate_actor(dealer, op, now)?;
        // Level 55 BjMotionControl.Init starts Normal.WaitStartLabel.
        self.script.motion_blend = false;
        self.leisure_motion("work_002.ks", "*待機", op, now)?;
        self.actors.apply(
            crate::kag::AdvUi::AllOffset {
                position: Some([5.25, 0.5, 0.]),
                rotation: Some([0., -90., 0.]),
            },
            &self.script,
            &self.files,
            &mut self.assets,
            op,
            now,
        )?;
        self.camera_control = false;
        self.adv_camera = NORMAL;
        self.sound
            .play(host, c, &self.files, Kind::Bgm, "BGM010.ogg", 0.5, true)?;
        let mut rows = self
            .assets
            .filtered_canvas(op, host, c, Blackjack.name(), "Canvas", |path| {
                path.is_empty() || path.starts_with("/BjUIGroup")
            })?
            .base;
        for w in &mut rows {
            if let Some(path) = w.path.strip_prefix("/BjUIGroup") {
                w.path = path.to_owned();
            }
        }
        for w in &rows {
            if w.path.starts_with("/ExChangeUI") || w.path.starts_with("/TopFlame") {
                self.leisure.base.push(w.clone());
            }
        }
        self.leisure.blackjack.panel = rows;
        self.draw_blackjack()?;
        Ok("CasinoMini".into())
    }

    fn draw_blackjack(&mut self) -> Result<()> {
        self.casino_widgets()?;
        if self.leisure.casino.exchange.is_some() {
            return Ok(());
        }
        let vm = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let s = &vm.runtime.host().state;
        let coins = s.retail.casino_coins;
        let table = &self.leisure.blackjack;
        let round = &table.round;
        let playing = !round.hands.is_empty();
        let result = !round.result.is_empty();
        let running = playing && !result;
        let warning_disabled = s.player.flag("ベット額警告非表示") != 0;
        let mut digits = [0; 3];
        for base in &table.panel {
            let p = base.path.as_str();
            let visible = if table.bet.is_some() {
                p.starts_with("/BetSetUI")
                    && !p.contains("CoinWarning")
                    && (!p.ends_with("/Checkmark") || warning_disabled)
            } else if p.starts_with("/ResultBG") {
                let indicator = round.result.first().map(|outcome| match outcome {
                    Outcome::Natural => "BlackJack",
                    Outcome::Win => "Win",
                    Outcome::Draw => "Draw",
                    Outcome::Loss => "Lose",
                    Outcome::Surrender => "Surrender",
                });
                result
                    && p.strip_prefix("/ResultBG/Result/")
                        .is_none_or(|n| Some(n) == indicator)
            } else if p.starts_with("/ScoreTextPanel") || p.starts_with("/BetTextpanel") {
                playing
                    && !p.ends_with("OutText")
                    && (!p.contains("Split") || round.hands.len() == 2)
            } else if p.starts_with("/CallUIPanel") {
                running
            } else {
                p.starts_with("/Frame")
                    && (!p.ends_with("/StartButton") || !playing)
                    && (!p.ends_with("/AngleChange") || running)
            };
            if !visible {
                continue;
            }
            let name = p.rsplit('/').next().unwrap_or_default();
            let mut w = base.clone();
            if let Some(action) = action(name)
                && p.starts_with("/CallUIPanel")
            {
                if round.permits(action, coins) {
                    w.collider = Some(w.rect);
                } else {
                    w.collider = None;
                    w.tint[3] = 100;
                }
            } else if matches!(
                name,
                "StartButton"
                    | "EndButton"
                    | "AngleChange"
                    | "Reset"
                    | "Max"
                    | "OK"
                    | "Cancel"
                    | "Next"
                    | "Background"
            ) && (!p.ends_with("/Background") || p.contains("/DialogCheck/"))
            {
                w.collider = Some(w.rect);
            }
            if let Some(bet) = table.bet
                && let Some(kind) = ["Up", "Down", "Number"].iter().position(|n| *n == name)
            {
                ensure!(digits[kind] < 5, "unexpected blackjack bet digit");
                let scale = 10i64.pow(4 - digits[kind]);
                digits[kind] += 1;
                w.path = format!("blackjack-bet/{name}/{scale}");
                if let Some(label) = w.label.as_mut() {
                    label.set_text(&(bet / scale % 10).to_string());
                } else {
                    w.collider = Some(w.rect);
                }
            } else if let Some(label) = w.label.as_mut() {
                let text = match name {
                    "Credit" => daily::money(coins - table.bet.unwrap_or(0)),
                    "CurrentMoney" => daily::money(coins),
                    "GetMoneyText" => format!("{:+}", round.change),
                    "ScoreText" if playing => hand_label(&round.hands[0]),
                    "SplitScoreText" if round.hands.len() == 2 => hand_label(&round.hands[1]),
                    "DealerScoreText" if result => {
                        format!("{}  {}", score(&round.dealer), cards(&round.dealer))
                    }
                    "DealerScoreText" if running => format!("{}  ▣", round.dealer[0].display()),
                    "BetText" if playing => daily::money(round.hands[0].bet),
                    "SplitBetText" if round.hands.len() == 2 => daily::money(round.hands[1].bet),
                    _ => {
                        self.widgets.push(w);
                        continue;
                    }
                };
                label.set_text(&text);
            }
            self.widgets.push(w);
        }
        Ok(())
    }

    pub(super) fn blackjack_bet(
        &mut self,
        warn: bool,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<()> {
        let bet = self.leisure.blackjack.bet.context("blackjack bet panel")?;
        if warn && bet >= 10_000 {
            return self.leisure_prompt(
                op,
                host,
                c,
                now,
                "10,000枚以上賭けようとしています。\n本当に賭けますか?",
                "okcancel",
                Response::BlackjackBet(false),
            );
        }
        {
            let mut vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let s = &mut vm.runtime.host_mut().state;
            let mut coins = s.retail.casino_coins;
            self.leisure
                .blackjack
                .round
                .start(bet, &mut coins, |n| s.random(n))?;
            s.retail.casino_coins = coins;
        }
        self.leisure.blackjack.bet = None;
        self.blackjack_result()?;
        self.draw_blackjack()
    }

    fn blackjack_result(&mut self) -> Result<()> {
        let round = &self.leisure.blackjack.round;
        if !round.result.is_empty() && round.change > 0 {
            let mut vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let s = &mut vm.runtime.host_mut().state;
            let total = s.player.flag("コイン入手総額").saturating_add(i32::try_from(round.change)?);
            s.player.generic_flag.insert("コイン入手総額".into(), total);
            if total >= 10_000 {
                vm.eval("SetNewTrophy(1120)")?;
            }
        }
        Ok(())
    }

    pub(super) fn blackjack_click(
        &mut self,
        i: usize,
        path: &str,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<()> {
        if path.starts_with("/TopFlame/")
            || path.starts_with("/ExChangeUI/")
            || path.starts_with("exchange/")
        {
            self.casino_click(i, path, op, host, c, now)?;
            if self.leisure.dialog.is_none() {
                self.draw_blackjack()?;
            }
            return Ok(());
        }
        let (coins, warn) = {
            let vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let s = &vm.runtime.host().state;
            (
                s.retail.casino_coins,
                s.player.flag("ベット額警告非表示") == 0,
            )
        };
        let table = &mut self.leisure.blackjack;
        let max = MAX.min(coins);
        match path {
            "/Frame/StartButton" if coins < MIN => {
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
            "/Frame/StartButton" => table.bet = Some(MIN),
            "/Frame/AngleChange" => {
                table.zoom = !table.zoom;
                self.adv_camera = if table.zoom { ZOOM } else { NORMAL };
            }
            "/Frame/EndButton" => {
                return self.leisure_prompt(
                    op,
                    host,
                    c,
                    now,
                    "カジノ画面に戻ります",
                    "okcancel",
                    Response::BlackjackExit,
                );
            }
            "/ResultBG/Next" => {
                table.round.clear();
                table.zoom = false;
                self.adv_camera = NORMAL;
            }
            "/BetSetUI/Max" => table.bet = Some(max),
            "/BetSetUI/Reset" => table.bet = Some(MIN),
            "/BetSetUI/Cancel" => table.bet = None,
            "/BetSetUI/DialogCheck/Background" => {
                let mut vm = self
                    .script
                    .tjs
                    .lock()
                    .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                vm.runtime
                    .host_mut()
                    .state
                    .player
                    .generic_flag
                    .insert("ベット額警告非表示".into(), i32::from(warn));
            }
            "/BetSetUI/OK" => {
                if warn && table.bet.is_some_and(|v| v * 2 > coins) {
                    return self.leisure_prompt(op, host, c, now, "現在の賭け額と所持コインの枚数では\nダブルダウン・スプリットは行えません\n本当に賭けますか?", "okcancel", Response::BlackjackBet(true));
                }
                return self.blackjack_bet(warn, op, host, c, now);
            }
            p if let Some((dir, scale)) = p
                .strip_prefix("blackjack-bet/")
                .and_then(|v| v.split_once('/'))
                && let Some(bet) = table.bet =>
            {
                let scale = scale.parse::<i64>()?;
                table.bet = Some(
                    if dir == "Up" {
                        bet + scale
                    } else {
                        bet - scale
                    }
                    .clamp(MIN, max),
                );
            }
            p if let Some(action) = p.strip_prefix("/CallUIPanel/").and_then(action) => {
                {
                    let mut vm = self
                        .script
                        .tjs
                        .lock()
                        .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                    table
                        .round
                        .play(action, &mut vm.runtime.host_mut().state.retail.casino_coins)?;
                }
                self.blackjack_result()?;
            }
            _ => {}
        }
        self.draw_blackjack()
    }
}

fn action(name: &str) -> Option<Action> {
    Some(match name {
        "Hit" => Action::Hit,
        "Stand" => Action::Stand,
        "Surrender" => Action::Surrender,
        "DoubleDown" => Action::Double,
        "Split" => Action::Split,
        _ => return None,
    })
}

fn cards(cards: &[Card]) -> String {
    cards
        .iter()
        .map(|c| c.display())
        .collect::<Vec<_>>()
        .join(" ")
}

fn hand_label(hand: &Hand) -> String {
    format!("{}  {}", hand.score(), cards(&hand.cards))
}
