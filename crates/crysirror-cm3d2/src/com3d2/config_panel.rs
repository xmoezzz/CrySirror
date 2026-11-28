// SPDX-License-Identifier: MPL-2.0
//! ConfigCtrl input and CMSystem settings for the standalone profile.
use super::*;

#[derive(Clone, Copy, PartialEq)]
enum Control {
    Volume(Option<Kind>),
    MessageTransparency,
    Bloom,
    PlayerOpacity,
    TextSpeed,
    AutoSpeed,
    DanceVolume,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Switch {
    Shortcuts,
    Fullscreen,
    Fps,
    SequentialVoice,
    SpecialSe,
    Spatial(Kind),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    Resolution,
    Samples,
    Shadows,
    Textures,
    Capture,
}
const SWITCHES: [(&str, Switch); 5] = [
    ("System/SysButtonShowAlways", Switch::Shortcuts),
    ("Screen/FullScreen", Switch::Fullscreen),
    ("Screen/FpsDisplay", Switch::Fps),
    ("Message/SequentialVoice", Switch::SequentialVoice),
    ("Sound/EjaculationSeEnabled", Switch::SpecialSe),
];
const SPATIAL: [(&str, Kind); 4] = [
    ("Sound/MusicVolume/3D", Kind::Bgm),
    ("Sound/VoiceVolume/3D", Kind::Voice),
    ("Sound/SoundEffectVolume/3D", Kind::Se),
    ("Sound/SoundEnvironmentVolume/3D", Kind::Env),
];
const MENUS: [(&str, Menu); 5] = [
    ("Resolution", Menu::Resolution),
    ("AntiAlias", Menu::Samples),
    ("ShadowQuality", Menu::Shadows),
    ("TextureQuality", Menu::Textures),
    ("CaptureMagnification", Menu::Capture),
];
pub(super) struct Settings {
    pub shortcuts: bool,
    fullscreen: bool,
    pub fps: bool,
    pub sequential_voice: bool,
    resolution: Option<[u32; 2]>,
    pub samples: u32,
    pub shadows: usize,
    pub anisotropy: usize,
    capture: usize,
    pub bloom: f32,
    pub player_opacity: f32,
    pub text_speed: f32,
    pub auto_speed: f32,
    pub auto: bool,
    pub line_at: u64,
    pub revealed: bool,
    pub control_press: Option<String>,
    pub shortcuts_open: bool,
    pub shortcut_base: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            shortcuts: true,
            fullscreen: false,
            fps: false,
            sequential_voice: true,
            resolution: None,
            samples: 2,
            shadows: 2,
            anisotropy: 2,
            capture: 0,
            bloom: 50.,
            player_opacity: 50.,
            text_speed: 80.,
            auto_speed: 50.,
            auto: false,
            line_at: 0,
            revealed: false,
            control_press: None,
            shortcuts_open: false,
            shortcut_base: false,
        }
    }
}
impl Settings {
    pub fn visible_prefix(&self, now: u64) -> usize {
        if self.revealed || self.char_delay() == 0 {
            usize::MAX
        } else {
            (1 + now.saturating_sub(self.line_at) / self.char_delay()) as usize
        }
    }

    pub fn char_delay(&self) -> u64 {
        ((100. - self.text_speed) * 1.5) as u64
    }
    pub fn auto_delay(&self) -> u64 {
        ((100. - self.auto_speed) * 30.) as u64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ButtonAction {
    Set(Switch, bool),
    Flip(Switch),
    Menu(usize),
}
struct Button {
    widget: usize,
    rect: Rect,
    action: ButtonAction,
    opacity: u8,
}
struct Dropdown {
    kind: Menu,
    widget: usize,
    label: usize,
    entries: Vec<String>,
}
struct Popup {
    first: usize,
    menu: usize,
    rows: Vec<Rect>,
    pressed: Option<usize>,
}

impl Game {
    fn switch_value(&self, kind: Switch) -> bool {
        match kind {
            Switch::Shortcuts => self.settings.shortcuts,
            Switch::Fullscreen => self.settings.fullscreen,
            Switch::Fps => self.settings.fps,
            Switch::SequentialVoice => self.settings.sequential_voice,
            Switch::SpecialSe => self.sound.special_se,
            Switch::Spatial(kind) => self.sound.spatial_enabled(kind),
        }
    }

    fn set_switch(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        kind: Switch,
        value: bool,
    ) -> Result<()> {
        match kind {
            Switch::Shortcuts => {
                self.settings.shortcuts = value;
                self.settings.shortcuts_open = false;
            }
            Switch::Fullscreen => {
                host.submit_engine(
                    client,
                    1,
                    &serde_json::to_vec(&json!({
                        "ui":[{"op":"set_fullscreen","enabled":value}]
                    }))?,
                )?;
                self.settings.fullscreen = value;
            }
            Switch::Fps => self.settings.fps = value,
            Switch::SequentialVoice => self.settings.sequential_voice = value,
            Switch::SpecialSe => self.sound.special_se = value,
            Switch::Spatial(kind) => self.sound.set_spatial_mode(host, client, kind, value)?,
        }
        Ok(())
    }

    fn apply_menu(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        menu: &Dropdown,
        choice: usize,
        resolutions: &[[u32; 2]],
    ) -> Result<()> {
        match menu.kind {
            Menu::Resolution => {
                let size = resolutions[choice];
                host.submit_engine(
                    client,
                    1,
                    &serde_json::to_vec(&json!({
                        "ui":[{"op":"set_window_size","size":size}]
                    }))?,
                )?;
                self.settings.resolution = Some(size);
            }
            Menu::Samples => {
                self.settings.samples = [1, 2, 4, 8][choice];
                self.apply_environment(host, client)?;
            }
            Menu::Shadows => {
                self.settings.shadows = choice;
                self.apply_environment(host, client)?;
            }
            Menu::Textures => {
                self.settings.anisotropy = choice;
                self.apply_environment(host, client)?;
            }
            Menu::Capture => {
                host.submit_engine(
                    client,
                    1,
                    &serde_json::to_vec(&json!({
                        "ui":[{"op":"set_capture_scale","scale":([1, 2, 4][choice])}]
                    }))?,
                )?;
                self.settings.capture = choice;
            }
        }
        self.widgets[menu.label]
            .label
            .as_mut()
            .unwrap()
            .set_text(&menu.entries[choice]);
        Ok(())
    }

    fn slider_value(&self, control: Control) -> f32 {
        match control {
            Control::Volume(None) => self.sound.all_volume,
            Control::Volume(Some(k)) => self.sound.volumes[k as usize],
            Control::MessageTransparency => self.message_transparency,
            Control::Bloom => self.settings.bloom,
            Control::PlayerOpacity => self.settings.player_opacity,
            Control::TextSpeed => self.settings.text_speed,
            Control::AutoSpeed => self.settings.auto_speed,
            Control::DanceVolume => self.sound.dance_volume,
        }
    }

    fn set_slider(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        control: Control,
        value: f32,
    ) -> Result<()> {
        if self.slider_value(control) == value {
            return Ok(());
        }
        match control {
            Control::Volume(k) => self.sound.set_volume(host, client, k, value)?,
            Control::MessageTransparency => self.message_transparency = value,
            Control::Bloom => {
                self.settings.bloom = value;
                self.apply_environment(host, client)?;
            }
            Control::PlayerOpacity => {
                self.settings.player_opacity = value;
                self.actors.set_player_opacity(value / 100.);
                self.editor.set_player_opacity(value / 100.);
            }
            Control::TextSpeed => self.settings.text_speed = value,
            Control::AutoSpeed => self.settings.auto_speed = value,
            Control::DanceVolume => self.sound.set_dance_volume(host, client, value)?,
        }
        Ok(())
    }
}
const SLIDERS: [(&str, Control); 12] = [
    ("Screen/Bloom", Control::Bloom),
    ("Screen/PlayerModelTransparency", Control::PlayerOpacity),
    ("Message/TextSpeed", Control::TextSpeed),
    ("Message/AutoSpeed", Control::AutoSpeed),
    ("Sound/DanceVolume", Control::DanceVolume),
    ("Sound/MasterVolume", Control::Volume(None)),
    ("Sound/SystemVolume", Control::Volume(Some(Kind::System))),
    ("Sound/MusicVolume", Control::Volume(Some(Kind::Bgm))),
    ("Sound/VoiceVolume", Control::Volume(Some(Kind::Voice))),
    ("Sound/SoundEffectVolume", Control::Volume(Some(Kind::Se))),
    (
        "Sound/SoundEnvironmentVolume",
        Control::Volume(Some(Kind::Env)),
    ),
    (
        "Message/MessageWindowTransparency",
        Control::MessageTransparency,
    ),
];
const ROOT: &str = "SystemUI Root/ConfigPanel/";
type Rect = [f32; 4];
fn inside(rect: Rect, input: InputFrame) -> bool {
    let (x, y) = (input.pointer_x as f32, input.pointer_y as f32);
    x >= rect[0] && x < rect[0] + rect[2] && y >= rect[1] && y < rect[1] + rect[3]
}
struct Slider {
    control: Control,
    track: Rect,
    hit: Rect,
    thumb: usize,
}
pub(super) struct Panel {
    first: usize,
    opened: u64,
    closing: Option<u64>,
    ok: Rect,
    ok_pressed: bool,
    drag: Option<usize>,
    sliders: Vec<Slider>,
    buttons: Vec<Button>,
    menus: Vec<Dropdown>,
    resolutions: Vec<[u32; 2]>,
    pressed: Option<usize>,
    popup: Option<Popup>,
}
impl Panel {
    pub fn first(&self) -> usize {
        self.first
    }

    pub fn opacity(&self, index: usize, now: u64) -> Option<u8> {
        if index < self.first {
            return None;
        }
        let fade = |start: u64| (now.saturating_sub(start) as f32 / 300.).min(1.);
        Some(
            (255.
                * self
                    .closing
                    .map_or_else(|| fade(self.opened), |start| 1. - fade(start)))
            .round() as u8,
        )
    }
}
impl Game {
    pub(super) fn open_config(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        now: u64,
    ) -> Result<()> {
        let mut rows = self.assets.scene(op, host, client, "StandaloneConfig")?;
        let first = self.widgets.len();
        let floor = rows
            .iter()
            .map(|w| w.depth)
            .min()
            .context("empty config panel")?;
        let top = self
            .widgets
            .iter()
            .map(|w| w.depth)
            .max()
            .unwrap_or_default()
            + 1;
        for w in &mut rows {
            w.depth = top + (w.depth - floor);
        }
        let mut config_buttons = Vec::new();
        let add_button =
            |buttons: &mut Vec<Button>, rows: &[Widget], path: String, action| -> Result<usize> {
                let widget = rows
                    .iter()
                    .position(|w| w.path == path)
                    .with_context(|| format!("missing config control {path}"))?;
                let control = &rows[widget];
                let rect = control
                    .collider
                    .with_context(|| format!("config control {path} has no collider"))?;
                buttons.push(Button {
                    widget: first + widget,
                    rect,
                    action,
                    opacity: control.tint[3],
                });
                Ok(widget)
            };
        for (path, kind) in SWITCHES {
            for (name, value) in [("On", true), ("Off", false)] {
                add_button(
                    &mut config_buttons,
                    &rows,
                    format!("{ROOT}{path}/{name}"),
                    ButtonAction::Set(kind, value),
                )?;
            }
        }
        for (path, kind) in SPATIAL {
            add_button(
                &mut config_buttons,
                &rows,
                format!("{ROOT}{path}"),
                ButtonAction::Flip(Switch::Spatial(kind)),
            )?;
        }
        let desktop = host.display_info().map(|d| d.screen);
        let current = host
            .window_geometry(client)?
            .map(|w| w.inner_size)
            .filter(|size| size.iter().all(|n| *n > 0));
        let requested = self.settings.resolution.or(current).unwrap_or([1280, 720]);
        let mut resolutions = [[1024, 576], [1280, 720], [1366, 768], [1920, 1080]]
            .into_iter()
            .filter(|size| desktop.is_none_or(|d| size[0] <= d[0] && size[1] <= d[1]))
            .collect::<Vec<_>>();
        for size in desktop.into_iter().chain(current).chain([requested]) {
            if size.into_iter().all(|n| (1..=16384).contains(&n)) && !resolutions.contains(&size) {
                resolutions.push(size);
            }
        }
        let mut menus = Vec::new();
        for (index, (name, kind)) in MENUS.into_iter().enumerate() {
            let path = format!("{ROOT}Screen/{name}/PopupList");
            let widget = add_button(
                &mut config_buttons,
                &rows,
                path.clone(),
                ButtonAction::Menu(index),
            )?;
            let label = rows
                .iter()
                .position(|w| w.path == format!("{path}/Label"))
                .with_context(|| format!("missing selection label {path}"))?;
            let (entries, selected) = match kind {
                Menu::Resolution => (
                    resolutions
                        .iter()
                        .map(|[w, h]| format!("{w}x{h}"))
                        .collect::<Vec<_>>(),
                    resolutions
                        .iter()
                        .position(|size| *size == requested)
                        .unwrap_or(0),
                ),
                Menu::Samples => (
                    names(&["None", "X2", "X4", "X8"]),
                    self.settings.samples.ilog2() as usize,
                ),
                Menu::Shadows => (
                    names(&["None", "Low", "Medium", "High"]),
                    self.settings.shadows,
                ),
                Menu::Textures => (names(&["Low", "Medium", "High"]), self.settings.anisotropy),
                Menu::Capture => (names(&["X1", "X2", "X4"]), self.settings.capture),
            };
            rows[label]
                .label
                .as_mut()
                .context("config dropdown needs UILabel")?
                .set_text(&entries[selected]);
            menus.push(Dropdown {
                kind,
                widget: first + widget,
                label: first + label,
                entries,
            });
        }
        let ok = rows
            .iter()
            .find(|w| w.path.ends_with("/ConfigPanel/OK"))
            .and_then(|w| w.collider)
            .context("missing system config OK")?;
        let mut sliders = vec![];
        for (path, control) in SLIDERS {
            let path = format!("{ROOT}{path}/Slider");
            let bar = rows
                .iter()
                .find(|w| w.path == path)
                .with_context(|| format!("missing {path}"))?;
            let hit = bar.collider.context("config slider has no collider")?;
            let track = rows
                .iter()
                .find(|w| w.path == format!("{path}/Foreground"))
                .map_or(bar.rect, |w| w.rect);
            ensure!(track[2] > 0., "empty config track {path}");
            let thumb = rows
                .iter()
                .position(|w| w.path == format!("{path}/Thumb"))
                .context("missing config slider thumb")?;
            let value = self.slider_value(control);
            let w = &mut rows[thumb];
            business::move_by(
                w,
                track[0] + track[2] * value / 100. - (w.rect[0] + w.rect[2] / 2.),
                0.,
            );
            sliders.push(Slider {
                control,
                track,
                hit,
                thumb: first + thumb,
            });
        }
        self.widgets.extend(rows);
        let panel = Panel {
            first,
            opened: now,
            closing: None,
            ok,
            ok_pressed: false,
            drag: None,
            sliders,
            buttons: config_buttons,
            menus,
            resolutions,
            pressed: None,
            popup: None,
        };
        self.color_config_buttons(&panel);
        self.config_panel = Some(panel);
        self.pressed = None;
        Ok(())
    }

    // The native shortcut panel is hover-revealed when SysButtonShowAlways is off.
    pub(super) fn settings_input(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        input: InputFrame,
        released: u64,
        now: u64,
    ) -> Result<bool> {
        if self.leaving.is_some()
            || self.choices.is_some()
            || self.quit_dialog.is_some()
            || self.slot_viewer.is_some()
            || self.business.dialog().is_some()
            || self.leisure.dialog.is_some()
            || self.power.dialog.is_some()
            || self.shop.dialog.is_some()
            || self.stock_editor.confirming.is_some()
            || self
                .construction
                .as_ref()
                .is_some_and(|s| s.dialog.is_some())
        {
            return Ok(false);
        }
        let has_message =
            self.scene == Scene::Script && self.message_open && self.dialogue.is_some();
        // The retail hit area extends beyond the icon strip in the top-right.
        self.settings.shortcuts_open = inside([1510., 0., 410., 104.], input);
        let hovered = self.widgets.iter().find_map(|widget| {
            let path = widget.path.as_str();
            let is_message = has_message
                && (path.ends_with("/MsgParent/Buttons/Auto")
                    || path.ends_with("/MsgParent/Buttons/Config"));
            let is_shortcut = (self.settings.shortcuts || self.settings.shortcuts_open)
                && (path == "SystemUI Root/SystemShortcut/Gear"
                    || self.settings.shortcut_base
                        && path == "SystemUI Root/SystemShortcut/Base/Grid/Config");
            ((is_message || is_shortcut) && widget.collider.is_some_and(|r| inside(r, input)))
                .then_some(path)
        });
        let pressed = input.buttons_pressed & buttons::MOUSE_LEFT != 0;
        if pressed {
            self.settings.control_press = hovered.map(str::to_owned);
        }
        if released & buttons::MOUSE_LEFT != 0
            && let Some(previous) = self.settings.control_press.take()
        {
            if hovered == Some(previous.as_str()) {
                if previous.ends_with("/Gear") {
                    self.settings.shortcut_base = !self.settings.shortcut_base;
                } else if previous.ends_with("/Auto") {
                    self.settings.auto = !self.settings.auto;
                } else {
                    self.open_config(op, host, client, now)?;
                }
            }
            return Ok(true);
        }
        Ok(self.settings.control_press.is_some())
    }

    pub(super) fn message_advance(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        pressed: bool,
        now: u64,
    ) -> Result<bool> {
        if self.choices.is_some() || self.script.hitret_since().is_none() {
            return Ok(pressed);
        }
        let Some((_, text)) = &self.dialogue else {
            return Ok(pressed);
        };
        let count =
            unicode_segmentation::UnicodeSegmentation::graphemes(text.as_str(), true).count();
        if pressed && !self.settings.sequential_voice {
            self.sound.stop(host, client, Kind::Voice, 0.)?;
        }
        if self.settings.visible_prefix(now) < count {
            if pressed {
                self.settings.revealed = true;
                self.settings.line_at = now;
            }
            return Ok(false);
        }
        let reveal_ms = if self.settings.revealed {
            0
        } else {
            (count.saturating_sub(1) as u64) * self.settings.char_delay()
        };
        let done_at = self.settings.line_at + reveal_ms;
        let auto = self.settings.auto
            && self.message_open
            && now.saturating_sub(done_at) >= self.settings.auto_delay()
            && (!self.settings.sequential_voice || !self.sound.voice_busy(host, client)?);
        if auto && !pressed && !self.settings.sequential_voice {
            self.sound.stop(host, client, Kind::Voice, 0.)?;
        }
        Ok(pressed || auto)
    }

    pub(super) fn update_voice_position(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
    ) -> Result<()> {
        let local = self
            .voice_maid
            .and_then(|(man, slot)| self.actors.voice_source(man, slot))
            .map_or([0.; 3], |position| {
                let (target, distance, [yaw, pitch]) = self.adv_camera;
                let rotation = glam::Quat::from_rotation_y(yaw.to_radians())
                    * glam::Quat::from_rotation_x(pitch.to_radians());
                let relative = rotation.inverse()
                    * (glam::Vec3::from_array(position) - glam::Vec3::from_array(target));
                [relative.x, relative.y, distance + relative.z]
            });
        self.sound.set_voice_position(host, client, local)
    }

    fn color_config_buttons(&mut self, panel: &Panel) {
        for button in &panel.buttons {
            let selected = match button.action {
                ButtonAction::Set(kind, value) => self.switch_value(kind) == value,
                ButtonAction::Flip(kind) => self.switch_value(kind),
                ButtonAction::Menu(_) => false,
            };
            self.widgets[button.widget].tint[3] = if selected { 255 } else { button.opacity };
        }
    }

    fn show_config_popup(&mut self, panel: &mut Panel, index: usize) {
        let menu = &panel.menus[index];
        let template = self.widgets[menu.widget].clone();
        let label = self.widgets[menu.label].clone();
        let top = self
            .widgets
            .iter()
            .map(|w| w.depth)
            .max()
            .unwrap_or_default()
            + 1;
        let first = self.widgets.len();
        let stride = template.rect[3];
        let total = stride * menu.entries.len() as f32;
        let y = if template.rect[1] + stride + total <= 1080. {
            template.rect[1] + stride
        } else {
            (template.rect[1] - total).max(0.)
        };
        let mut rects = Vec::new();
        for (i, value) in menu.entries.iter().enumerate() {
            let mut background = template.clone();
            let mut item = label.clone();
            let dy = y + i as f32 * stride - template.rect[1];
            business::move_by(&mut background, 0., dy);
            business::move_by(&mut item, 0., dy);
            background.depth = top;
            item.depth = top + 1;
            item.label.as_mut().unwrap().set_text(value);
            rects.push(background.rect);
            self.widgets.extend([background, item]);
        }
        panel.popup = Some(Popup {
            first,
            menu: index,
            rows: rects,
            pressed: None,
        });
    }

    pub(super) fn update_config(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        input: InputFrame,
        released: u64,
        now: u64,
    ) -> Result<()> {
        let mut panel = self.config_panel.take().context("missing config panel")?;
        if let Some(start) = panel.closing
            && now.saturating_sub(start) >= 300
        {
            self.widgets.truncate(panel.first);
            return Ok(());
        }
        let result = if panel.closing.is_none() {
            self.config_input(&mut panel, host, client, input, released, now)
        } else {
            Ok(())
        };
        self.config_panel = Some(panel);
        result
    }

    fn config_input(
        &mut self,
        panel: &mut Panel,
        host: &mut dyn HostServices,
        client: ClientId,
        input: InputFrame,
        released: u64,
        now: u64,
    ) -> Result<()> {
        let pressed = input.buttons_pressed & buttons::MOUSE_LEFT != 0;
        let released = released & buttons::MOUSE_LEFT != 0;
        if let Some(popup) = &mut panel.popup {
            let hit = popup.rows.iter().position(|&r| inside(r, input));
            if pressed {
                popup.pressed = hit;
            }
            if released || input.buttons_pressed & buttons::ESCAPE != 0 {
                let popup = panel.popup.take().unwrap();
                self.widgets.truncate(popup.first);
                if released && let Some(choice) = hit.filter(|&i| Some(i) == popup.pressed) {
                    self.apply_menu(
                        host,
                        client,
                        &panel.menus[popup.menu],
                        choice,
                        &panel.resolutions,
                    )?;
                }
            }
            return Ok(());
        }
        let hit = panel.buttons.iter().position(|b| inside(b.rect, input));
        if pressed {
            panel.ok_pressed = inside(panel.ok, input);
            panel.pressed = hit;
            panel.drag = panel
                .sliders
                .iter()
                .position(|s| inside(s.hit, input) || inside(self.widgets[s.thumb].rect, input));
        }
        if let Some(i) = panel.drag
            && (released || input.buttons_down & buttons::MOUSE_LEFT != 0)
        {
            let s = &panel.sliders[i];
            let value =
                (((input.pointer_x as f32 - s.track[0]) / s.track[2]).clamp(0., 1.) * 100.).ceil();
            self.set_slider(host, client, s.control, value)?;
            let w = &mut self.widgets[s.thumb];
            business::move_by(
                w,
                s.track[0] + s.track[2] * value / 100. - (w.rect[0] + w.rect[2] / 2.),
                0.,
            );
        }
        if released {
            if panel.ok_pressed && inside(panel.ok, input) {
                panel.closing = Some(now);
            } else if let Some(i) =
                hit.filter(|&i| panel.pressed == Some(i) && panel.drag.is_none())
            {
                match panel.buttons[i].action {
                    ButtonAction::Set(kind, value) => self.set_switch(host, client, kind, value)?,
                    ButtonAction::Flip(kind) => {
                        self.set_switch(host, client, kind, !self.switch_value(kind))?
                    }
                    ButtonAction::Menu(index) => self.show_config_popup(panel, index),
                }
                self.color_config_buttons(panel);
            }
            panel.ok_pressed = false;
            panel.pressed = None;
            panel.drag = None;
        }
        Ok(())
    }
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
#[ignore = "requires installed standalone system UI and title"]
fn installed_title_config_modal_sound_and_return() -> Result<()> {
    let mut m = installed_menu::Menu::new()?;
    m.title()?;
    let config = m
        .game
        .widgets
        .iter()
        .find(|w| w.path.ends_with("/ConfigGroup/Config"))
        .context("title config button")?
        .rect;
    m.click(config)?;
    m.tick(300)?;
    let panel = m
        .game
        .config_panel
        .as_ref()
        .context("title did not open config")?;
    let slider = panel
        .sliders
        .iter()
        .find(|s| s.control == Control::Volume(None))
        .unwrap();
    let [x, y, width, height] = slider.track;
    let quarter = [x + width * 0.25, y, 1., height];
    m.click(quarter)?;
    assert!((24.0..=26.).contains(&m.game.sound.all_volume));
    let changed = m.game.sound.all_volume;
    let start = m
        .game
        .widgets
        .iter()
        .find(|w| w.path.ends_with("/StartGroup/Start"))
        .context("title start button")?
        .rect;
    m.click(start)?;
    assert!(m.game.leaving.is_none());
    let ok = m.game.config_panel.as_ref().unwrap().ok;
    m.click(ok)?;
    m.tick(300)?;
    assert!(m.game.config_panel.is_none());
    assert_eq!(m.game.scene, Scene::Title);
    m.click(config)?;
    let slider = m
        .game
        .config_panel
        .as_ref()
        .unwrap()
        .sliders
        .iter()
        .find(|s| s.control == Control::Volume(None))
        .unwrap();
    let thumb = m.game.widgets[slider.thumb].rect;
    assert!((thumb[0] + thumb[2] / 2. - (x + width * changed / 100.)).abs() < 0.1);
    Ok(())
}

#[test]
#[ignore = "requires the installed 1.0.4 title and system UI"]
fn installed_title_config_remaining_controls_publish_engine_state() -> Result<()> {
    let mut m = installed_menu::Menu::new()?;
    m.title()?;
    let config = m
        .game
        .widgets
        .iter()
        .find(|w| w.path.ends_with("/ConfigGroup/Config"))
        .unwrap()
        .rect;
    m.click(config)?;
    m.tick(300)?;
    m.rt.host.presentation_fps = Some(60);
    let panel = m.game.config_panel.as_ref().context("panel did not open")?;
    assert_eq!((panel.buttons.len(), panel.sliders.len()), (19, 12));
    let click = |m: &mut installed_menu::Menu, action| {
        let r = m
            .game
            .config_panel
            .as_ref()
            .unwrap()
            .buttons
            .iter()
            .find(|b| b.action == action)
            .unwrap()
            .rect;
        m.click(r)
    };
    for kind in [
        Switch::Shortcuts,
        Switch::Fullscreen,
        Switch::Fps,
        Switch::SequentialVoice,
        Switch::SpecialSe,
    ] {
        for enabled in [false, true, false] {
            click(&mut m, ButtonAction::Set(kind, enabled))?;
            assert_eq!(m.game.switch_value(kind), enabled, "{kind:?}");
            if kind == Switch::Fullscreen {
                assert_eq!(m.rt.host.engine.ui.fullscreen, Some((m.client.0, enabled)));
            }
            if kind == Switch::Fps {
                let count = m.game.previous_layers as u32;
                let layer = m.rt.host.engine.layers.get(m.client, count).unwrap();
                assert_eq!(
                    serde_json::to_value(&layer.paint)?["text"] == "60 FPS",
                    enabled
                );
            }
        }
    }
    for (_, kind) in SPATIAL {
        let old = m.game.sound.spatial_enabled(kind);
        click(&mut m, ButtonAction::Flip(Switch::Spatial(kind)))?;
        assert_eq!(m.game.sound.spatial_enabled(kind), !old);
    }
    for (menu, choice) in [(0, 0), (1, 0), (2, 0), (3, 1), (4, 1)] {
        click(&mut m, ButtonAction::Menu(menu))?;
        let panel = m.game.config_panel.as_ref().unwrap();
        let popup = panel.popup.as_ref().context("missing dropdown")?;
        let label = panel.menus[menu].label;
        let expected = panel.menus[menu].entries[choice].clone();
        m.click(popup.rows[choice])?;
        assert!(m.game.config_panel.as_ref().unwrap().popup.is_none());
        assert_eq!(m.game.widgets[label].label.as_ref().unwrap().text, expected);
    }
    let engine = &m.rt.host.engine;
    assert_eq!(
        engine.ui.window_requests.size,
        Some((m.client.0, [1024, 576]))
    );
    assert_eq!(engine.ui.capture_scale, Some((m.client.0, 2)));
    let (_, env) = engine.world.view.environment.as_ref().unwrap();
    assert!(!env.sun.as_ref().unwrap().shadows);
    let quality = env.raster_quality.unwrap();
    assert_eq!((quality.samples, quality.anisotropy), (1, 4));

    for control in [
        Control::Bloom,
        Control::PlayerOpacity,
        Control::TextSpeed,
        Control::AutoSpeed,
        Control::DanceVolume,
    ] {
        let slider = m
            .game
            .config_panel
            .as_ref()
            .unwrap()
            .sliders
            .iter()
            .find(|s| s.control == control)
            .unwrap();
        let [x, y, w, h] = slider.track;
        m.click([x + w * 0.75, y, 1., h])?;
        assert!((74.0..=76.).contains(&m.game.slider_value(control)));
    }
    let (_, env) = m.rt.host.engine.world.view.environment.as_ref().unwrap();
    assert!((0.059..0.061).contains(&env.bloom));
    assert!((36..40).contains(&m.game.settings.char_delay()));
    assert!((720..=780).contains(&m.game.settings.auto_delay()));
    let group =
        m.rt.host
            .audio_mix_group(m.client, "com3d2-dance")?
            .unwrap();
    assert!((0.74..=0.76).contains(&group.settings.gain));
    Ok(())
}

#[test]
#[ignore = "requires the installed 1.0.4 opening script and UI"]
fn installed_config_text_speed_drives_reveal_and_auto() -> Result<()> {
    let mut m = installed_menu::Menu::new()?;
    m.open_script("OP_0001a.ks", "*L0", Scene::Script)?;
    let (full, start) = {
        let (_, text) = m.game.dialogue.as_ref().context("missing opening line")?;
        (text.to_owned(), m.game.settings.line_at)
    };
    let settings = &mut m.game.settings;
    settings.text_speed = 0.;
    settings.auto_speed = 0.;
    settings.sequential_voice = false;
    let first = m.game.script.position();
    let now = m.rt.now_ms();
    assert!(settings.visible_prefix(start) < full.graphemes(true).count());
    assert!(
        !m.game
            .message_advance(&mut m.rt.host, m.client, true, start)?
    );
    assert_eq!(m.game.script.position(), first);
    assert_eq!(m.game.settings.visible_prefix(now), usize::MAX);
    m.game.settings.auto = true;
    assert!(
        !m.game
            .message_advance(&mut m.rt.host, m.client, false, start + 2999)?
    );
    assert!(
        m.game
            .message_advance(&mut m.rt.host, m.client, false, start + 3000)?
    );
    Ok(())
}
