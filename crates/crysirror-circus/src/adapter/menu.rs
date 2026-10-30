//! Original process11 group/action policy. Motion, coverage, hit testing and
//! retained visual state are engine capabilities; this module only maps them.
use super::*;
use crysirror_core::image_processing::{ImageInput, ImageProcessState};
const ROOT: u32 = 0x36ff_fffe;
const UI_ROOT: u32 = 0x36ff_fffd;
const BASE: u32 = 0x36ff_0000;
pub(super) struct NativeMenu {
    group: Group,
    ticks: u64,
    /// Per-button original visual state99/1/2 and scale transition admission.
    states: Vec<i16>,
    scale_started: BTreeMap<u32, u64>,
    pending_sounds: Vec<u16>,
    pending_action: Option<String>,
    active: bool,
}
impl Bridge<'_> {
    pub(super) fn clear_menu(&mut self) -> Result<()> {
        if self.services.menu.take().is_some() {
            self.submit(
                vec![json!({"op":"remove","id":ROOT})],
                vec![json!({"op":"remove","id":UI_ROOT})],
            )?;
        }
        Ok(())
    }

    pub(super) fn begin_menu(&mut self, group: &Group) -> Result<bool> {
        if self.services.menu.as_ref().is_some_and(|m| m.active) {
            return Ok(true);
        }
        if self.services.menu.take().is_some() {
            self.submit(
                vec![json!({"op":"remove","id":ROOT})],
                vec![json!({"op":"remove","id":UI_ROOT})],
            )?;
        }
        ensure!(
            group.buttons.len() <= 224 && group.images.len() <= 4096,
            "Circus retained menu declaration limit"
        );
        ensure!(
            group.image_animations.is_empty(),
            "Circus menu image-animation activation unsupported"
        );
        ensure!(
            group
                .buttons
                .iter()
                .all(|b| b.texts.iter().all(|t| t.text.is_empty()))
                && group.texts.iter().all(|t| t.text.is_empty()),
            "Circus menu nonempty native text policy unsupported"
        );
        // Acquire immutable CPU leases once through shared image workers. The
        // empty operation graph preserves the original stored alpha bytes.
        for button in &group.buttons {
            if !button.behavior_flags.contains(&213) {
                continue;
            }
            let Some(part) = button
                .images
                .iter()
                .find(|p| p.state == 99 && self.services.images.contains_key(&p.slot))
            else {
                continue;
            };
            if self.services.menu_images.contains_key(&part.slot) {
                continue;
            }
            if let Some((slot, job)) = &mut self.services.menu_image_job {
                ensure!(*slot == part.slot, "Circus menu image completion scope");
                match job.poll()? {
                    ImageProcessState::Pending => return Ok(false),
                    ImageProcessState::Failed(e) => bail!("Circus menu image lease failed: {e}"),
                    ImageProcessState::Ready(image) => {
                        ensure!(
                            image.client() == self.client,
                            "Circus foreign image coverage"
                        );
                        self.services.menu_images.insert(part.slot, image);
                        self.services.menu_image_job = None;
                    }
                }
            } else {
                let name = self
                    .services
                    .image_names
                    .get(&part.slot)
                    .context("Circus menu image source")?;
                let source = ImageInput::Asset {
                    path: image_path(name)?.relative().into(),
                    size: self.services.image_extents[&part.slot],
                    fingerprint: None,
                };
                let job = self.host.image_metadata(self.client)?.begin_process(
                    ImageInput::Composite {
                        base: Box::new(source),
                        layers: vec![],
                    },
                    Arc::from(b"[]".as_slice()),
                )?;
                self.services.menu_image_job = Some((part.slot, job));
                return Ok(false);
            }
        }
        self.root()?;
        let [x, y] = group.position;
        let mut layers = vec![
            json!({"op":"put","layer":{"id":ROOT,"parent":SCENE_ROOT,"rect":[x,y,1280,720],"paint":{"kind":"solid","color":[0,0,0,0]},"order":[100,0]}}),
        ];
        let mut ui = vec![
            json!({"op":"put","widget":{"id":UI_ROOT,"rect":[x,y,1280,720],"interaction_only":true}}),
        ];
        for (n, part) in group.images.iter().enumerate() {
            let id = BASE + n as u32;
            self.image_layer(&mut layers, id, part, 100)?;
            if let Some(last) = layers.last_mut().filter(|v| v["layer"]["id"] == id) {
                last["layer"]["parent"] = json!(ROOT);
                last["layer"]["visible"] = json!(true);
            }
        }
        for (n, button) in group.buttons.iter().enumerate() {
            let [x, y, r, b] = button.bounds;
            ensure!(r > x && b > y, "Circus button extent");
            let widget = BASE + 0x8000 + n as u32;
            ui.push(json!({"op":"put","widget":{"id":widget,"parent":UI_ROOT,"rect":[x,y,r-x,b-y],"kind":"button","interaction_only":true,"keyboard_focusable":false}}));
            if button.behavior_flags.contains(&213) {
                if let Some(part) = button
                    .images
                    .iter()
                    .find(|p| p.state == 99 && self.services.menu_images.contains_key(&p.slot))
                {
                    let resource = &self.services.menu_images[&part.slot];
                    let sx = u32::try_from(part.source[0])?;
                    let sy = u32::try_from(part.source[1])?;
                    let size = [u32::try_from(r - x)?, u32::try_from(b - y)?];
                    let coverage = resource.alpha_region([sx, sy, size[0], size[1]])?;
                    ui.push(json!({"op":"set_hit_mask","id":widget,"mask":{"size":size,"coverage":coverage,"threshold":1,"strict_bounds":true}}));
                }
            }
            for (k, part) in button.images.iter().enumerate() {
                let id = BASE + 4096 + n as u32 * 128 + k as u32;
                self.image_layer(&mut layers, id, part, 100)?;
                if let Some(last) = layers.last_mut().filter(|v| v["layer"]["id"] == id) {
                    last["layer"]["parent"] = json!(ROOT);
                    last["layer"]["visible"] = json!(part.state == 99);
                }
            }
        }
        if let Some(m) = &group.motion {
            for (property, from, to) in [("x", m.from[0], m.to[0]), ("y", m.from[1], m.to[1])] {
                if from != to {
                    let track = json!({"property":property,"from":from,"to":to,"duration":m.duration_ticks,"external":true});
                    layers.push(json!({"op":"begin_motion","id":ROOT,"track":track}));
                    ui.push(json!({"op":"begin_motion","id":UI_ROOT,"track":track}));
                }
            }
        }
        // The original process starts with no pending previous-control action.
        // Engine events are retained across worker suspensions; transient hover
        // state alone must never serve as an activation mailbox.
        let _ = self.host.query_engine(self.client, EngineQuery::UiEvents)?;
        self.submit(layers, ui)?;
        self.services.menu = Some(NativeMenu {
            group: group.clone(),
            ticks: 0,
            states: vec![99; group.buttons.len()],
            scale_started: BTreeMap::new(),
            pending_sounds: Vec::new(),
            pending_action: None,
            active: true,
        });
        Ok(true)
    }
    pub(super) fn advance_menu(&mut self) -> Result<()> {
        let Some(menu) = self.services.menu.as_mut().filter(|m| m.active) else {
            return Ok(());
        };
        menu.ticks += 1;
        let mut layers = Vec::new();
        let mut ui = Vec::new();
        if let Some(m) = &menu.group.motion {
            for (property, from, to) in [("x", m.from[0], m.to[0]), ("y", m.from[1], m.to[1])] {
                if from != to {
                    layers.push(json!({"op":"set_motion_elapsed","id":ROOT,"property":property,"elapsed":menu.ticks}));
                    ui.push(json!({"op":"set_motion_elapsed","id":UI_ROOT,"property":property,"elapsed":menu.ticks}));
                }
            }
        }
        for (&id, &started) in &menu.scale_started {
            for property in ["scale_x", "scale_y"] {
                layers.push(json!({"op":"set_motion_elapsed","id":id,"property":property,"elapsed":menu.ticks.saturating_sub(started)}));
            }
        }
        self.submit(layers, ui)
    }
    pub(super) fn tick_menu(&mut self) -> Result<Option<String>> {
        let mut menu = self
            .services
            .menu
            .take()
            .context("Circus process11 without menu")?;
        let result = self.tick_menu_inner(&mut menu);
        self.services.menu = Some(menu);
        result
    }
    fn tick_menu_inner(&mut self, menu: &mut NativeMenu) -> Result<Option<String>> {
        ensure!(
            self.input.buttons_pressed & !crysirror_core::service::buttons::MOUSE_LEFT == 0,
            "Circus process11 keyboard/controller/page navigation unsupported"
        );
        if menu.pending_action.is_none() {
            let events: Vec<Value> = serde_json::from_slice(
                &self.host.query_engine(self.client, EngineQuery::UiEvents)?,
            )?;
            let activated = events
                .iter()
                .filter(|e| e["kind"] == "activate")
                .filter_map(|e| e["widget"]["id"].as_u64())
                .collect::<BTreeSet<_>>();
            let mut layers = Vec::new();
            for (n, button) in menu.group.buttons.iter().enumerate() {
                let widget = BASE + 0x8000 + n as u32;
                let status: Value = serde_json::from_slice(
                    &self
                        .host
                        .query_engine(self.client, EngineQuery::UiInteraction(widget))?,
                )?;
                let hovered = status["hovered"]
                    .as_bool()
                    .context("Circus menu UI hover state")?;
                let pressed = status["pressed"]
                    .as_bool()
                    .context("Circus menu UI press state")?;
                let activated = activated.contains(&u64::from(widget));
                let state = if pressed {
                    2
                } else if hovered {
                    1
                } else {
                    99
                };
                if state != menu.states[n] {
                    if menu.states[n] == 99 && state != 99 && button.sounds[1] != 0 {
                        menu.pending_sounds.push(button.sounds[1].into());
                    }
                    let transition = (menu.states[n] == 99) != (state == 99);
                    for (k, part) in button.images.iter().enumerate() {
                        if !self.services.images.contains_key(&part.slot)
                            || part.source[2] == 0
                            || part.source[3] == 0
                        {
                            continue;
                        }
                        let id = BASE + 4096 + n as u32 * 128 + k as u32;
                        layers
                            .push(json!({"op":"set_visible","id":id,"visible":part.state==state}));
                        if transition && menu.group.behavior_flags.contains(&242) {
                            let (from, to) = if state == 99 { (1.2, 1.) } else { (1., 1.2) };
                            for property in ["scale_x", "scale_y"] {
                                layers.push(json!({"op":"begin_motion","id":id,"track":{"property":property,"from":from,"to":to,"duration":15,"external":true}}));
                            }
                            menu.scale_started.insert(id, menu.ticks);
                        }
                    }
                    menu.states[n] = state;
                }
                if activated && !button.action.is_empty() && button.action != "-" {
                    ensure!(
                        menu.pending_action.is_none(),
                        "Circus ambiguous menu activation"
                    );
                    menu.pending_action = Some(button.action.clone());
                    if button.sounds[0] != 0 {
                        menu.pending_sounds.push(button.sounds[0].into());
                    }
                }
            }
            ensure!(
                menu.pending_sounds.len() <= 32,
                "Circus menu sound queue limit"
            );
            self.submit(layers, vec![])?;
        }
        if let Some(&sound) = menu.pending_sounds.first() {
            let Some(name) = self.sound_name(sound)? else {
                return Ok(None);
            };
            if !self.play_audio(2, &name, false, false, 0, None)? {
                return Ok(None);
            }
            menu.pending_sounds.remove(0);
            if !menu.pending_sounds.is_empty() {
                return Ok(None);
            }
        }
        if let Some(action) = menu.pending_action.take() {
            self.submit(
                vec![],
                vec![json!({"op":"set_enabled","id":UI_ROOT,"enabled":false})],
            )?;
            menu.active = false;
            return Ok(Some(action));
        }
        Ok(None)
    }
}
