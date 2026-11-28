// SPDX-License-Identifier: MPL-2.0
//! Standalone level 56. Translate Reel UVs, credits and uGUI callbacks.
use super::*;
use crate::unity_scene::ObjectKey;
use crysirror_unity::{PPtr, class_id, reader::Reader};

mod lights;
mod round;
use lights::Lighting;
use round::{Payout, Reach, ReelMarks};

const MACHINE: u32 = 0x5600;
const WHITE_FADE: u32 = 65533;
const TUTORIAL: &str = "/Canvas Tutorial Panel/";
const HELP: &str = "SystemUI Root/SystemShortcut/Base/Grid/Help";
const REELS: [[usize; 21]; 3] = [
    [
        3, 0, 4, 5, 4, 5, 1, 6, 5, 4, 5, 0, 2, 5, 4, 5, 6, 1, 5, 4, 5,
    ],
    [
        4, 0, 5, 6, 4, 3, 5, 6, 4, 1, 5, 6, 2, 4, 0, 5, 6, 4, 3, 5, 6,
    ],
    [
        5, 0, 1, 3, 4, 5, 2, 3, 4, 5, 2, 3, 4, 5, 2, 3, 4, 5, 2, 3, 4,
    ],
];
const LINES: [[i32; 3]; 5] = [[0; 3], [1; 3], [-1; 3], [1, 0, -1], [-1, 0, 1]];
const STOPS: [&str; 3] = [
    "/StopButtons/Button1",
    "/StopButtons/Button2",
    "/StopButtons/Button3",
];
const SYMBOLS: [&str; 7] = [
    "スリーセブン",
    "BAR",
    "ピエロ",
    "ベル",
    "メダル",
    "ぶどう",
    "チェリー",
];

fn symbol(reel: usize, position: i32, row: i32) -> usize {
    REELS[reel][(21 - (position + 9 + row)).rem_euclid(21) as usize]
}

/// References from the serialized SceneCasinoSlot component in level 56.
struct Layout {
    machine: String,
    level: String,
    lever: String,
    lever_parent_rotation: glam::Quat,
    ball: String,
    top: String,
    win: String,
    heart: String,
    reel_lights: Vec<String>,
    stop_lamps: Vec<String>,
    bet_lamps: Vec<String>,
    line_lamps: Vec<String>,
    payout_digits: Vec<String>,
    credit_digits: Vec<String>,
    chance_lamps: Vec<String>,
    seven_lamps: Vec<String>,
    bonus_lamps: Vec<String>,
    surfaces: Vec<ObjectKey>,
    start_sprites: [ObjectKey; 2],
    all_lamps: Vec<String>,
}

fn references(r: &mut Reader<'_>, expected: usize) -> Result<Vec<PPtr>> {
    let count = r.count(12)?;
    ensure!(count == expected, "invalid slot-machine reference array");
    (0..count).map(|_| r.pptr64()).collect()
}

fn layout(assets: &mut Assets, op: &mut Opener) -> Result<(Layout, Orbit, String)> {
    let level = assets.scene_file(SlotMachine.name())?;
    let sf = assets.unity.file(op, &level)?;
    let mut controller = None;
    for entry in sf
        .objects
        .iter()
        .filter(|entry| entry.class_id == class_id::MONO_BEHAVIOUR)
    {
        let bytes = sf.bytes(entry)?;
        let mut r = Reader::new(&bytes);
        r.seek(16)?;
        let Some(script) = assets.unity.resolve(op, &level, r.pptr64()?)? else {
            continue;
        };
        let (_, script) = assets.unity.bytes(op, &script)?;
        if Reader::new(&script).string()? == "SceneCasinoSlot" {
            controller = Some(bytes);
            break;
        }
    }
    let bytes = controller.context("missing SceneCasinoSlot component")?;
    let mut r = Reader::new(&bytes);
    r.seek(32)?;
    let object = |reference: PPtr| -> Result<String> {
        ensure!(reference.file_id == 0, "external casino scene object");
        let bytes = sf.bytes_of(reference.path_id)?;
        let mut reader = Reader::new(&bytes);
        let count = reader.count(12)?;
        reader.take(count * 12 + 4)?;
        reader.string()
    };
    let cabinet = sf.bytes_of(r.pptr64()?.path_id)?;
    let mut go = Reader::new(&cabinet);
    ensure!(go.count(12)? > 0, "slot cabinet has no transform");
    let transform = sf.bytes_of(go.pptr64()?.path_id)?;
    let mut tr = Reader::new(&transform);
    tr.seek(52)?;
    let children = tr.count(12)?;
    tr.take(children * 12)?;
    let parent = sf.bytes_of(tr.pptr64()?.path_id)?;
    let mut parent = Reader::new(&parent);
    let machine = object(parent.pptr64()?)?;
    let lever_parent_rotation = glam::Quat::from_array(parent.f32s()?).normalize();
    let lever = object(r.pptr64()?)?;
    let ball = object(r.pptr64()?)?;
    let top = object(r.pptr64()?)?;
    let win = object(r.pptr64()?)?;
    let heart = object(r.pptr64()?)?;
    r.take(24)?; // uGUI start button and full-window fade mask.
    references(&mut r, 3)?; // Reel components themselves; UVs are on the children.
    let names = |r: &mut Reader<'_>, size| -> Result<Vec<String>> {
        references(r, size)?.into_iter().map(object).collect()
    };
    let reel_lights = names(&mut r, 3)?;
    let stop_lamps = names(&mut r, 3)?
        .into_iter()
        .map(|name| name.replacen("Botton", "Slot_Botton", 1))
        .collect::<Vec<_>>();
    let bet_lamps = names(&mut r, 3)?;
    let line_lamps = names(&mut r, 5)?;
    let payout_digits = names(&mut r, 6)?;
    let credit_digits = names(&mut r, 6)?;
    let chance_lamps = names(&mut r, 10)?;
    let seven_lamps = names(&mut r, 3)?;
    let bonus_lamps = names(&mut r, 8)?;
    let mut surfaces = Vec::new();
    for texture in references(&mut r, 4)? {
        surfaces.push(
            assets
                .unity
                .resolve(op, &level, texture)?
                .context("missing slot panel surface")?,
        );
    }
    let mut start_sprites = Vec::new();
    for sprite in references(&mut r, 2)? {
        start_sprites.push(
            assets
                .unity
                .resolve(op, &level, sprite)?
                .context("missing slot start-button sprite")?,
        );
    }
    r.pptr64()?; // SlotManager.
    let camera = (r.f32s()?, r.f32()?, r.f32s()?);
    let bgm = r.string()?;
    let mut all_lamps = vec![ball.clone(), top.clone(), win.clone(), heart.clone()];
    for lamps in [
        &reel_lights,
        &stop_lamps,
        &bet_lamps,
        &line_lamps,
        &chance_lamps,
        &seven_lamps,
        &bonus_lamps,
    ] {
        all_lamps.extend(lamps.iter().cloned());
    }
    Ok((
        Layout {
            machine,
            level,
            lever,
            lever_parent_rotation,
            ball,
            top,
            win,
            heart,
            reel_lights,
            stop_lamps,
            bet_lamps,
            line_lamps,
            payout_digits,
            credit_digits,
            chance_lamps,
            seven_lamps,
            bonus_lamps,
            surfaces,
            start_sprites: start_sprites
                .try_into()
                .map_err(|_| anyhow::anyhow!("slot start sprites"))?,
            all_lamps,
        },
        camera,
        bgm,
    ))
}

struct ResultLights {
    start: u64,
    jackpot: bool,
    paylines: [bool; 5],
    chance_correction: i32,
    voice_played: bool,
    payout: Payout,
}

struct ChanceFade {
    start: u64,
    surface: usize,
    applied: bool,
}

struct Tutorial {
    opened: u64,
    closing: Option<u64>,
    widgets: Vec<Widget>,
}

#[derive(Default)]
pub(super) struct Machine {
    model: Option<String>,
    layout: Option<Layout>,
    lighting: Option<Lighting>,
    tutorial_pages: Vec<(String, String)>,
    tutorial: Option<Tutorial>,
    upload_slot: bool,
    surface: usize,
    intro_until: u64,
    exit_at: Option<u64>,
    fade: Option<ChanceFade>,
    fade_layer: bool,
    result: Option<ResultLights>,
    sliding: Option<usize>,
    last_stop: Option<usize>,
    reach: Reach,
    marks: ReelMarks,
    seven: [bool; 3],
    bet_at: Option<u64>,
    bet_sound_steps: usize,
    lever_sound: bool,
    payout_sound: bool,
    controls: Vec<Widget>,
    start_images: Option<[(String, [f32; 4]); 2]>,
    hovered_control: Option<&'static str>,
    input: Option<InputFrame>,
    payouts: [i64; 7],
    weights: [[usize; 7]; 2],
    max_probability: [usize; 2],
    pay_step_ms: u64,
    chance: bool,
    chance_rounds: u8,
    normal_rounds: u32,
    bar_pending: bool,
    target: Option<(usize, i32)>,
    stops: [Option<i32>; 3],
    offsets: [f32; 3],
    carry: [f32; 3],
    spin: Option<u64>,
    next_stop: u64,
    credit: i64,
    payout: i64,
}

impl Machine {
    fn busy(&self, now: u64) -> bool {
        self.spin.is_some()
            || self.result.is_some()
            || self.fade.is_some()
            || now < self.intro_until
            || self.exit_at.is_some()
            || self.tutorial.is_some()
    }
}

impl Game {
    pub(super) fn load_slot_machine(
        &mut self,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<String> {
        let mut machine = Machine::default();
        let mut show_tutorial = false;
        {
            let mut vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let s = &mut vm.runtime.host_mut().state;
            let t = s.retail.table("casino_slot_haitou.nei")?;
            for row in (1..t.rows).filter(|&row| t.exists(0, row)) {
                let name = t.string(1, row);
                if let Some(symbol) = SYMBOLS.iter().position(|n| *n == name) {
                    machine.payouts[symbol] = i64::from(t.int(2, row).max(0));
                    machine.weights[0][symbol] = t.int(3, row).max(0) as usize;
                    machine.weights[1][symbol] = t.int(4, row).max(0) as usize;
                    for mode in 0..2 {
                        machine.max_probability[mode] += machine.weights[mode][symbol];
                    }
                }
            }
            let delay = s.retail.table("casino_slot_byou.nei")?;
            machine.pay_step_ms = (1..delay.rows)
                .find(|&i| delay.exists(0, i))
                .map(|i| delay.int(0, i).max(0) as u64)
                .context("slot payout interval")?;
            if s.retail
                .casino_slot
                .is_none_or(|slot| !s.retail.casino_dealers.contains_key(&slot))
            {
                // SceneCasinoSlot.Start's standalone/direct-entry credit.
                s.retail.casino_coins = 499_999;
            }
            machine.credit = s.retail.casino_coins;
            let tutorial = s.retail.table("tutorial_list.nei")?;
            if let Some(row) =
                (1..tutorial.rows).find(|&row| tutorial.string(0, row) == SlotMachine.name())
            {
                machine.tutorial_pages = (2..tutorial.columns)
                    .step_by(2)
                    .filter(|&column| tutorial.exists(column, row))
                    .map(|column| {
                        (
                            tutorial.string(column, row),
                            tutorial.string(column + 1, row),
                        )
                    })
                    .collect();
                let flag = "__チュートリアルフラグ_SceneCasinoSlot";
                show_tutorial = !machine.tutorial_pages.is_empty() && s.player.flag(flag) <= 0;
            }
        }
        let rows = self
            .assets
            .filtered_canvas(op, h, c, SlotMachine.name(), "Canvas", |path| {
                path.is_empty()
                    || path.starts_with("/SlotGameUI")
                    || path.starts_with("/ExChangeUI")
            })?
            .base;
        for mut widget in rows {
            if let Some(path) = widget.path.strip_prefix("/SlotGameUI") {
                widget.path = path.to_owned();
            }
            if widget.path.starts_with("/TopFlame") || widget.path.starts_with("/ExChangeUI") {
                self.leisure.base.push(widget);
            } else if widget.path.starts_with("/StopButtons")
                || widget.path.starts_with("/BottomFlame")
            {
                machine.controls.push(widget);
            }
        }
        let (style, camera, bgm) = layout(&mut self.assets, op)?;
        let mut start_images = Vec::new();
        for sprite in &style.start_sprites {
            let (key, [x, y, width, height]) = self.assets.sprite(op, sprite.clone())?;
            let (asset, tw, th) = self.assets.texture(op, h, c, key)?;
            start_images.push((
                asset,
                [
                    x / tw as f32,
                    1. - (y + height) / th as f32,
                    (x + width) / tw as f32,
                    1. - y / th as f32,
                ],
            ));
        }
        machine.start_images = Some(
            start_images
                .try_into()
                .map_err(|_| anyhow::anyhow!("slot start images"))?,
        );
        machine.lighting = Some(Lighting::load(&style, &mut self.assets, op, h, c)?);
        machine.layout = Some(style);
        machine.surface = 3;
        machine.intro_until = now + 1000;
        self.adv_camera = camera;
        self.camera_control = true;
        self.main_light = MainLight::default();
        if !machine.tutorial_pages.is_empty() {
            self.leisure
                .base
                .extend(self.assets.scene(op, h, c, "SlotHelpShortcut")?);
        }
        self.leisure.machine = machine;
        self.apply_slot_surface(op, h, c, 3)?;
        self.sound
            .play(h, c, &self.files, Kind::Bgm, &bgm, 0., true)?;
        self.draw_slot_machine(now)?;
        if show_tutorial {
            self.open_slot_tutorial(op, h, c, now)?;
        }
        self.draw_slot_machine_model(h, c, now)?;
        Ok("CasinoMini".to_owned())
    }

    fn apply_slot_surface(
        &mut self,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
        index: usize,
    ) -> Result<()> {
        let machine = &mut self.leisure.machine;
        let layout = machine.layout.as_ref().context("slot machine layout")?;
        let (model, _) = club::model_variant(
            &mut self.assets.unity,
            op,
            &layout.level,
            &layout.machine,
            Some(("Slot_Panel", &layout.surfaces[index])),
            &layout.all_lamps,
        )?;
        let key = MACHINE + u32::from(!machine.upload_slot);
        let asset = h.upload_model(c, key, &model)?.relative().to_owned();
        let mut world = Vec::new();
        if machine.model.is_some() {
            world.push(json!({"op":"despawn","id":MACHINE}));
        }
        world.push(json!({"op":"spawn","entity":{"id":MACHINE,"name":"SlotMachine","geometry":{"kind":"scene","asset":asset}}}));
        h.submit_engine(c, 1, &serde_json::to_vec(&json!({"world":world}))?)?;
        if let Some(old) = machine.model.replace(asset) {
            h.release_model(
                c,
                &crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, &old)?,
            )?;
        }
        machine.upload_slot = !machine.upload_slot;
        machine.surface = index;
        Ok(())
    }

    pub(in crate::com3d2) fn release_slot_machine(
        &mut self,
        op: &Opener,
        h: &mut dyn HostServices,
        c: ClientId,
    ) -> Result<()> {
        if self.leisure.machine.fade_layer {
            h.submit_engine(
                c,
                1,
                &serde_json::to_vec(&json!({"layers":[{"op":"remove","id":WHITE_FADE}]}))?,
            )?;
        }
        if let Some(asset) = self.leisure.machine.model.take() {
            h.submit_engine(
                c,
                1,
                &serde_json::to_vec(&json!({"world":[{"op":"despawn","id":MACHINE}]}))?,
            )?;
            h.release_model(
                c,
                &crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, &asset)?,
            )?;
        }
        if let Some(lighting) = self.leisure.machine.lighting.take() {
            lighting.release(op, h, c)?;
        }
        Ok(())
    }

    fn draw_slot_machine(&mut self, now: u64) -> Result<()> {
        let pressed = self
            .leisure
            .pressed
            .and_then(|i| self.widgets.get(i))
            .map(|w| w.path.clone());
        self.casino_widgets()?;
        let machine = &self.leisure.machine;
        let available = !machine.busy(now);
        for widget in self.widgets.iter_mut().filter(|w| w.path == HELP) {
            widget.collider = (self.settings.shortcut_base
                && (self.settings.shortcuts || self.settings.shortcuts_open))
                .then_some(widget.rect);
        }
        if !available {
            for widget in self
                .widgets
                .iter_mut()
                .filter(|w| w.path == "/TopFlame/ButtonKankin")
            {
                widget.collider = None;
            }
        }
        if self.leisure.casino.exchange.is_some() {
            return Ok(());
        }
        self.widgets
            .extend(machine.controls.iter().cloned().map(|mut w| {
                let enabled = match w.path.as_str() {
                    "/BottomFlame/ButtonStart" | "/BottomFlame/ButtonEnd" => Some(available),
                    p => STOPS.iter().position(|button| *button == p).map(|reel| {
                        machine.spin.is_some_and(|start| now >= start)
                            && now >= machine.next_stop
                            && machine.stops[reel].is_none()
                    }),
                };
                if let Some(enabled) = enabled {
                    // The filtered canvas supplies the Image rectangle. These
                    // five installed objects have actual Unity Buttons on it.
                    w.collider = enabled.then_some(w.rect);
                    let color = if !enabled {
                        [128, 128, 128, 200]
                    } else if machine
                        .input
                        .is_some_and(|i| business::contains(w.rect, &i))
                    {
                        if pressed.as_deref() == Some(w.path.as_str())
                            && machine
                                .input
                                .is_some_and(|i| i.buttons_down & buttons::MOUSE_LEFT != 0)
                        {
                            [255, 255, 255, 170]
                        } else {
                            [255; 4]
                        }
                    } else {
                        [255, 255, 255, 200]
                    };
                    for (tint, color) in w.tint.iter_mut().zip(color) {
                        *tint = ((u16::from(*tint) * color + 127) / 255) as u8;
                    }
                }
                if w.path == "/BottomFlame/ButtonStart"
                    && let Some(images) = &machine.start_images
                {
                    let (asset, uv) =
                        &images[usize::from(machine.spin.is_some() || machine.result.is_some())];
                    w.asset = asset.clone();
                    w.uv = *uv;
                    w.slices = None;
                }
                w
            }));
        if let Some(tutorial) = &machine.tutorial {
            for w in &mut self.widgets {
                w.collider = None;
            }
            let allow_input = tutorial.closing.is_none() && now >= tutorial.opened + 500;
            self.widgets
                .extend(tutorial.widgets.iter().cloned().map(|mut w| {
                    if !allow_input {
                        w.collider = None;
                    }
                    w
                }));
        }
        Ok(())
    }

    pub(in crate::com3d2) fn slot_tutorial_opacity(&self, now: u64) -> u8 {
        self.leisure
            .machine
            .tutorial
            .as_ref()
            .map_or(255, |tutorial| {
                let time = if let Some(closing) = tutorial.closing {
                    500u64.saturating_sub(now.saturating_sub(closing))
                } else {
                    now.saturating_sub(tutorial.opened).min(500)
                };
                (time * 255 / 500) as u8
            })
    }

    pub(in crate::com3d2) fn open_slot_tutorial(
        &mut self,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<()> {
        let machine = &mut self.leisure.machine;
        if machine.tutorial_pages.is_empty() || machine.tutorial.is_some() {
            return Ok(());
        }
        machine.tutorial = Some(Tutorial {
            opened: now,
            closing: None,
            widgets: Vec::new(),
        });
        self.slot_tutorial_page(0, op, h, c)?;
        self.script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?
            .runtime
            .host_mut()
            .state
            .player
            .generic_flag
            .insert("__チュートリアルフラグ_SceneCasinoSlot".into(), 1);
        self.camera_control = false;
        self.draw_slot_machine(now)
    }

    fn slot_tutorial_page(
        &mut self,
        page: usize,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
    ) -> Result<()> {
        let machine = &mut self.leisure.machine;
        let mut widgets = self.leisure.base.clone();
        let first = widgets.len();
        self.assets.tutorial(
            op,
            h,
            c,
            &machine.tutorial_pages,
            page,
            &self.files,
            &mut widgets,
        )?;
        machine
            .tutorial
            .as_mut()
            .context("no slot tutorial")?
            .widgets = widgets.split_off(first);
        Ok(())
    }

    pub(super) fn tick_slot_machine(
        &mut self,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
        input: InputFrame,
        now: u64,
    ) -> Result<()> {
        if self.leisure.machine.exit_at.is_some_and(|exit| now >= exit)
            && self
                .script
                .tag_backup
                .get("cancel_label")
                .is_some_and(|label| !label.is_empty())
        {
            return self.resume_at("cancel_label", op, h, c, now);
        }
        if self
            .leisure
            .machine
            .tutorial
            .as_ref()
            .and_then(|t| t.closing)
            .is_some_and(|at| now >= at + 500)
        {
            self.leisure.machine.tutorial = None;
            self.camera_control = true;
        }
        let mut sounds = Vec::new();
        let mut stop_payout_sound = false;
        let mut surface = None;
        {
            let mut vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let s = &mut vm.runtime.host_mut().state;
            let m = &mut self.leisure.machine;
            if let Some(at) = m.bet_at {
                let age = now.saturating_sub(at);
                let steps = [400, 500, 600]
                    .into_iter()
                    .filter(|&delay| age >= delay)
                    .count();
                if steps > m.bet_sound_steps {
                    m.credit -= i64::try_from(steps - m.bet_sound_steps)? * 100;
                    m.bet_sound_steps = steps;
                    sounds.push(("casino_SE005.ogg", false));
                }
                if age >= 950 && !m.lever_sound {
                    sounds.push(("casino_SE006.ogg", false));
                    m.lever_sound = true;
                }
            }
            if m.sliding.is_some() && now >= m.next_stop {
                let has_reach = m.finish_stop(now, |max| s.random(max));
                if has_reach {
                    sounds.push(("casino_SE008.ogg", false));
                    if m.last_stop.is_some_and(|reel| m.seven[reel])
                        && let Some(voice) = ["H0_09451.ogg", "H1_09950.ogg", "H2_10068.ogg"]
                            .get(m.surface)
                            .copied()
                    {
                        sounds.push((voice, false));
                    }
                }
                if m.stops.iter().all(Option::is_some) {
                    m.settle(now);
                    if m.result.as_ref().is_some_and(|r| r.jackpot) {
                        sounds.push(("casino_SE009.ogg", false));
                    }
                }
            }
            if let Some(result) = &mut m.result
                && result.jackpot
                && !result.voice_played
                && now >= result.start + 6000
            {
                if let Some(file) = ["H0_09452.ogg", "H1_09951.ogg", "H2_10069.ogg"]
                    .get(m.surface)
                    .copied()
                {
                    sounds.push((file, false));
                }
                result.voice_played = true;
            }
            if let Some(result) = &mut m.result {
                let playing = result.payout.tick(
                    now,
                    &mut s.retail.casino_coins,
                    &mut m.credit,
                    &mut m.payout,
                );
                if playing && !m.payout_sound {
                    sounds.push(("casino_SE010.ogg", true));
                } else if !playing && m.payout_sound {
                    stop_payout_sound = true;
                }
                m.payout_sound = playing;
                if let Some(amount) = result.payout.trophy.take() {
                    crate::tjs::retail::record_casino_winnings(s, amount)?;
                }
            }
            if m.result.as_ref().is_some_and(|r| r.payout.done())
                && let Some(result) = m.result.take()
            {
                stop_payout_sound |= std::mem::take(&mut m.payout_sound);
                m.bet_at = None;
                m.reach = Reach::default();
                m.marks = ReelMarks::default();
                m.seven = [false; 3];
                let next_chance = if result.jackpot {
                    // EndChanceMode, then ChanceModeCheck with correction -1.
                    s.random(100);
                    false
                } else if m.chance {
                    m.chance_rounds < 15
                } else {
                    (s.random(100) as i32) < 1 + result.chance_correction
                };
                if next_chance != m.chance || result.jackpot {
                    m.chance = next_chance;
                    m.chance_rounds = 0;
                    m.normal_rounds = 0;
                    if m.chance {
                        sounds.push(("casino_SE011.ogg", false));
                        m.fade = Some(ChanceFade {
                            start: now,
                            surface: s.random(3),
                            applied: false,
                        });
                    } else {
                        surface = Some(3);
                    }
                }
            }
            if let Some(fade) = &mut m.fade {
                if now >= fade.start + 1500 && !fade.applied {
                    surface = Some(fade.surface);
                    fade.applied = true;
                }
                if now >= fade.start + 3000 {
                    m.fade = None;
                }
            }
            if !m.busy(now) {
                m.bet_at = None;
            }
        }
        if stop_payout_sound {
            self.sound.stop(h, c, Kind::Se, 0.)?;
        }
        for (file, looping) in sounds {
            if self.files.exists(file) {
                self.sound
                    .play(h, c, &self.files, Kind::Se, file, 0., looping)?;
            }
        }
        if let Some(index) = surface {
            self.apply_slot_surface(op, h, c, index)?;
        }
        self.leisure.machine.input = Some(input);
        if input.buttons_pressed & buttons::SPACE != 0 && self.leisure.casino.exchange.is_none() {
            let path = if self.leisure.machine.spin.is_some() {
                self.leisure
                    .machine
                    .stops
                    .iter()
                    .position(Option::is_none)
                    .map(|i| STOPS[i])
            } else if !self.leisure.machine.busy(now) {
                Some("/BottomFlame/ButtonStart")
            } else {
                None
            };
            if let Some(path) = path {
                self.slot_machine_click(path, op, h, c, now)?;
            }
        }
        self.draw_slot_machine(now)?;
        self.draw_slot_machine_model(h, c, now)?;
        let hovered = ["/BottomFlame/ButtonStart", "/BottomFlame/ButtonEnd"]
            .into_iter()
            .find(|path| {
                self.widgets.iter().any(|w| {
                    w.path == *path && w.collider.is_some_and(|r| business::contains(r, &input))
                })
            });
        let old = std::mem::replace(&mut self.leisure.machine.hovered_control, hovered);
        if hovered.is_some() && hovered != old {
            self.slot_system_sound(h, c, crate::sound::SeType::Hover)?;
        }
        Ok(())
    }

    fn slot_system_sound(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        event: crate::sound::SeType,
    ) -> Result<()> {
        let file = crate::sound::system_file(event);
        if self.files.exists(file) {
            self.sound
                .play(host, client, &self.files, Kind::System, file, 0., false)?;
        }
        Ok(())
    }

    pub(super) fn begin_slot_exit(&mut self, now: u64) {
        self.leisure.machine.exit_at = Some(now + 500);
    }

    fn draw_slot_machine_model(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<()> {
        let m = &mut self.leisure.machine;
        let layout = m.layout.as_ref().context("slot machine layout")?;
        let mut offsets = std::collections::BTreeMap::new();
        let phases = std::array::from_fn(|reel| m.phase(reel, now));
        for (reel, axis) in ['L', 'C', 'R'].into_iter().enumerate() {
            offsets.insert(format!("Slot_Reel{axis}"), [0., -phases[reel]]);
        }
        for (names, mut value) in [
            (&layout.credit_digits, m.credit),
            (&layout.payout_digits, m.payout),
        ] {
            for name in names {
                offsets.insert(name.clone(), [0., (value % 10) as f32 / 10.]);
                value /= 10;
            }
        }
        let age = m.bet_at.map_or(0, |at| now.saturating_sub(at));
        let mut lit = BTreeSet::new();
        if let Some(result) = &m.result {
            let elapsed = now.saturating_sub(result.start);
            let blink = |interval| elapsed / interval % 2 == 0;
            if blink(100) {
                lit.extend([&layout.top, &layout.ball, &layout.win]);
            }
            if blink(500) {
                lit.extend(&layout.bet_lamps);
            }
            for (i, lamp) in layout.line_lamps.iter().enumerate() {
                if result.jackpot || result.paylines[i] {
                    lit.insert(lamp);
                }
            }
            if result.jackpot {
                if blink(100) {
                    lit.insert(&layout.heart);
                }
                if let Some(index) = m.last_stop {
                    lit.insert(&layout.seven_lamps[index]);
                }
                for (i, lamp) in layout.bonus_lamps.iter().enumerate() {
                    let delay = i as u64 * 500;
                    if elapsed < delay || (elapsed - delay) % 2000 < 1500 {
                        lit.insert(lamp);
                    }
                }
            }
        } else if m.spin.is_some() {
            for (i, at) in [400, 500, 600].into_iter().enumerate() {
                if age >= at {
                    lit.insert(&layout.bet_lamps[i]);
                    let start = i.saturating_mul(2).saturating_sub(1);
                    lit.extend(&layout.line_lamps[start..i * 2 + 1]);
                }
            }
            for (i, lamp) in layout.stop_lamps.iter().enumerate() {
                if age >= 950 && m.stops[i].is_none() {
                    lit.insert(lamp);
                }
            }
        }
        if m.result.is_none() && m.reach.active() {
            lit.extend([&layout.top, &layout.ball]);
        }
        for (reel, lamp) in layout.seven_lamps.iter().enumerate() {
            if m.seven[reel] {
                lit.insert(lamp);
            }
        }
        if m.chance && m.fade.as_ref().is_none_or(|fade| fade.applied) {
            lit.extend(&layout.chance_lamps);
        }
        let lit = lit.into_iter().cloned().collect();
        let lever_time = (age.saturating_sub(700) as f32 / 250.).clamp(0., 1.);
        let motion = if m.bet_at.is_some() {
            1. - (2. * lever_time - 1.).abs()
        } else {
            0.
        };
        let from = crate::chara::unity_euler([-90., 90., 0.]);
        let to = crate::chara::unity_euler([-50., 90., 0.]);
        let local = layout.lever_parent_rotation.inverse() * from.slerp(to, motion);
        let rotation = crysirror_unity::gltf::mirror_quat(local.to_array());
        let mut nodes = std::collections::BTreeMap::new();
        nodes.insert(layout.lever.clone(), json!({"rotation":rotation}));
        let mut world = vec![
            json!({"op":"set_animation","id":MACHINE,"animation":{"uv_offsets":offsets,"nodes":nodes}}),
        ];
        m.lighting
            .as_mut()
            .context("slot lamps")?
            .animate(now, &lit, phases, &m.marks, &mut world);
        let fade = if let Some(exit) = m.exit_at {
            Some((
                (1. - exit.saturating_sub(now) as f32 / 500.).clamp(0., 1.),
                [0, 0, 0, 255],
            ))
        } else if now < m.intro_until {
            Some(((m.intro_until - now) as f32 / 1000., [0, 0, 0, 255]))
        } else if let Some(fade) = &m.fade {
            let age = now.saturating_sub(fade.start) as f32;
            Some(((1. - (age / 1500. - 1.).abs()).clamp(0., 1.), [255; 4]))
        } else {
            None
        };
        let mut layers = Vec::new();
        if let Some((opacity, color)) = fade {
            let opacity = (opacity * 255.).round() as u8;
            layers.push(
                json!({"op":"put","layer":{"id":WHITE_FADE,"rect":[0,0,1920,1080],
                "opacity":opacity,"order":[100010,0],"paint":{"kind":"solid","color":color}}}),
            );
        } else if m.fade_layer {
            layers.push(json!({"op":"remove","id":WHITE_FADE}));
        }
        h.submit_engine(
            c,
            1,
            &serde_json::to_vec(&json!({"world":world,"layers":layers}))?,
        )?;
        m.fade_layer = fade.is_some();
        Ok(())
    }

    pub(super) fn slot_machine_click(
        &mut self,
        path: &str,
        op: &mut Opener,
        h: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<()> {
        if let Some(tutorial) = &mut self.leisure.machine.tutorial {
            if tutorial.closing.is_none() && path.starts_with(TUTORIAL) {
                if path.ends_with("/Button OK") {
                    tutorial.closing = Some(now);
                } else if let Some((_, page)) = path.split_once("/page-")
                    && let Some((page, _)) = page.split_once('/')
                {
                    self.slot_tutorial_page(page.parse()?, op, h, c)?;
                }
                self.draw_slot_machine(now)?;
            }
            return Ok(());
        }
        if path == HELP {
            return self.open_slot_tutorial(op, h, c, now);
        }
        if path.starts_with("/TopFlame/")
            || path.starts_with("/ExChangeUI/")
            || path.starts_with("exchange/")
        {
            if self.leisure.casino.exchange.is_none() && self.leisure.machine.busy(now) {
                return Ok(());
            }
            let was_open = self.leisure.casino.exchange.is_some();
            self.casino_click(0, path, op, h, c, now)?;
            if was_open && self.leisure.casino.exchange.is_none() {
                self.leisure.machine.credit = self
                    .script
                    .tjs
                    .lock()
                    .map_err(|_| anyhow::anyhow!("TJS lock"))?
                    .runtime
                    .host()
                    .state
                    .retail
                    .casino_coins;
            }
            return self.draw_slot_machine(now);
        }
        if self.leisure.casino.exchange.is_some() {
            return Ok(());
        }
        if path == "/BottomFlame/ButtonEnd" && !self.leisure.machine.busy(now) {
            self.slot_system_sound(h, c, crate::sound::SeType::Click)?;
            return self.leisure_prompt(
                op,
                h,
                c,
                now,
                "カジノ画面に戻ります",
                "okcancel",
                Response::SlotExit,
            );
        }
        if path == "/BottomFlame/ButtonStart" && !self.leisure.machine.busy(now) {
            self.slot_system_sound(h, c, crate::sound::SeType::Click)?;
        }
        let mut vm = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let state = &mut vm.runtime.host_mut().state;
        let m = &mut self.leisure.machine;
        if path == "/BottomFlame/ButtonStart" && !m.busy(now) {
            if state.retail.casino_coins < 300 {
                drop(vm);
                return self.leisure_prompt(
                    op,
                    h,
                    c,
                    now,
                    "スロットをするには最低でも\nコインが300枚必用です",
                    "ok",
                    Response::Dismiss,
                );
            }
            state.retail.casino_coins -= 300;
            m.spin = Some(now + 950);
            m.bet_at = Some(now);
            m.bet_sound_steps = 0;
            m.lever_sound = false;
            m.sliding = None;
            m.last_stop = None;
            m.stops = [None; 3];
            m.target = None;
            m.next_stop = now + 950;
            if m.chance {
                m.chance_rounds += 1;
            } else {
                m.normal_rounds = m.normal_rounds.saturating_add(1);
            }
        } else if let Some(reel) = STOPS.iter().position(|p| *p == path)
            && m.spin.is_some_and(|start| now >= start)
            && now >= m.next_stop
            && m.stops[reel].is_none()
        {
            m.stop(reel, now);
            self.sound
                .play(h, c, &self.files, Kind::Se, "casino_SE007.ogg", 0., false)?;
        }
        Ok(())
    }
}
