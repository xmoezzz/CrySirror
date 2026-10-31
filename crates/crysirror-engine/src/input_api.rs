// SPDX-License-Identifier: MPL-2.0
//! Session-owned action maps, device-independent rebinding and prompt tokens.
//! Hardware handles remain in the desktop backend; only checked events cross here.
use crate::{
    context::EngineContext,
    object::{Class, ClassInfo, MethodInfo, ObjectId, PropertyInfo, TypeHint as T},
    value::Value,
};
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::service::{buttons, InputFrame};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

const MAX_ACTIONS: usize = 256;
const MAX_BINDINGS: usize = 16;
fn one() -> f32 {
    1.
}
fn deadzone() -> f32 {
    0.15
}
fn threshold() -> f32 {
    0.5
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GamepadFamily {
    #[default]
    Generic,
    Xbox,
    Playstation,
    Nintendo,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PadButton {
    South,
    East,
    West,
    North,
    LeftShoulder,
    RightShoulder,
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    Guide,
    LeftStick,
    RightStick,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PadAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    LeftTrigger,
    RightTrigger,
    DpadX,
    DpadY,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Binding {
    Key {
        key: String,
    },
    Button {
        button: PadButton,
    },
    Axis {
        axis: PadAxis,
        #[serde(default = "one")]
        scale: f32,
        #[serde(default = "deadzone")]
        dead_zone: f32,
    },
}
impl Binding {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Key { key } => {
                key_bit(key)?;
            }
            Self::Axis {
                scale, dead_zone, ..
            } => ensure!(
                scale.is_finite()
                    && scale.abs() > 0.
                    && scale.abs() <= 4.
                    && dead_zone.is_finite()
                    && (0. ..1.).contains(dead_zone),
                "invalid action axis scale/dead zone"
            ),
            _ => (),
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionBinding {
    pub bindings: Vec<Binding>,
    #[serde(default = "threshold")]
    pub threshold: f32,
}
impl ActionBinding {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.bindings.is_empty() && self.bindings.len() <= MAX_BINDINGS,
            "action requires 1..16 bindings"
        );
        ensure!(
            self.threshold.is_finite() && self.threshold > 0. && self.threshold <= 1.,
            "action threshold must be 0..1 exclusive of zero"
        );
        for b in &self.bindings {
            b.validate()?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingProfile {
    pub schema: u32,
    pub actions: BTreeMap<String, ActionBinding>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RebindResult {
    pub action: String,
    pub binding: Binding,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prompt {
    pub device: String,
    pub family: GamepadFamily,
    pub control: String,
    pub label: String,
    pub icon: String,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GamepadEvent {
    Connected {
        id: u32,
        family: GamepadFamily,
    },
    Disconnected {
        id: u32,
    },
    Button {
        id: u32,
        button: PadButton,
        value: f32,
    },
    Axis {
        id: u32,
        axis: PadAxis,
        value: f32,
    },
}
#[derive(Default)]
struct PadState {
    family: GamepadFamily,
    buttons: BTreeMap<PadButton, f32>,
    axes: BTreeMap<PadAxis, f32>,
}
/// Session configuration is explicitly exportable/importable. Hardware state is
/// not serialized and cannot reappear as held keys after loading a profile.
#[derive(Default)]
pub struct ActionMap {
    /// Legacy keyboard masks remain for source compatibility, not persistence.
    pub bindings: BTreeMap<String, u64>,
    pub previous_down: u64,
    pub previous_pointer: (i32, i32),
    actions: BTreeMap<String, ActionBinding>,
    frame: InputFrame,
    previous_values: BTreeMap<String, f32>,
    pads: BTreeMap<u32, PadState>,
    active_pad: Option<u32>,
    gamepad_prompts: bool,
    pending_gamepad_activity: bool,
    prompt_family: Option<GamepadFamily>,
    pending_rebind: Option<String>,
    rebind_result: Option<RebindResult>,
    suppress_edges: BTreeSet<String>,
    pressed_buttons: BTreeMap<PadButton, (f32, f32)>,
    pending_pressed_buttons: BTreeMap<PadButton, (f32, f32)>,
    navigation_previous: u64,
    navigation_presses: u64,
}
fn name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
}
fn key_bit(key: &str) -> Result<u64> {
    buttons::NAMES
        .iter()
        .find_map(|(n, b)| (*n == key).then_some(*b))
        .with_context(|| format!("unknown key '{key}'"))
}
fn token<T: Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .expect("enum serialization")
        .as_str()
        .expect("enum string")
        .to_owned()
}
impl ActionMap {
    pub fn rebind(&mut self, action: &str, spec: ActionBinding) -> Result<()> {
        ensure!(name_ok(action), "invalid action name");
        spec.validate()?;
        ensure!(
            self.actions.contains_key(action) || self.actions.len() < MAX_ACTIONS,
            "action map limit"
        );
        if self.actions.contains_key(action) {
            self.suppress_edges.insert(action.into());
        }
        let bits = spec
            .bindings
            .iter()
            .filter_map(|b| {
                if let Binding::Key { key } = b {
                    key_bit(key).ok()
                } else {
                    None
                }
            })
            .fold(0, |a, b| a | b);
        self.bindings.insert(action.into(), bits);
        self.actions.insert(action.into(), spec);
        Ok(())
    }
    pub fn bind(&mut self, action: &str, keys: &[String]) -> Result<()> {
        self.rebind(
            action,
            ActionBinding {
                bindings: keys
                    .iter()
                    .map(|k| Binding::Key { key: k.clone() })
                    .collect(),
                threshold: threshold(),
            },
        )
    }
    pub fn binding(&self, action: &str) -> Result<&ActionBinding> {
        self.actions
            .get(action)
            .with_context(|| format!("action '{action}' is not bound"))
    }
    pub fn export(&self) -> BindingProfile {
        BindingProfile {
            schema: 1,
            actions: self.actions.clone(),
        }
    }
    pub fn import(&mut self, profile: BindingProfile) -> Result<()> {
        ensure!(
            profile.schema == 1 && profile.actions.len() <= MAX_ACTIONS,
            "invalid input profile version/limit"
        );
        for (name, binding) in &profile.actions {
            ensure!(name_ok(name), "invalid action name");
            binding.validate()?;
        }
        // Validate the complete replacement before changing anything.
        self.actions = profile.actions;
        self.bindings.clear();
        for (name, spec) in &self.actions {
            self.bindings.insert(
                name.clone(),
                spec.bindings
                    .iter()
                    .filter_map(|b| {
                        if let Binding::Key { key } = b {
                            key_bit(key).ok()
                        } else {
                            None
                        }
                    })
                    .fold(0, |a, b| a | b),
            );
        }
        self.previous_values.clear();
        self.suppress_edges = self.actions.keys().cloned().collect();
        self.pending_rebind = None;
        self.rebind_result = None;
        Ok(())
    }
    pub fn begin_frame(&mut self, input: InputFrame) {
        let keyboard_activity = input.buttons_pressed != 0
            || input.buttons_down != self.previous_down
            || (input.pointer_x, input.pointer_y) != self.previous_pointer;
        if keyboard_activity && !self.pending_gamepad_activity {
            self.gamepad_prompts = false;
        }
        self.pending_gamepad_activity = false;
        self.frame = input;
        self.pressed_buttons = std::mem::take(&mut self.pending_pressed_buttons);
        if self.pending_rebind.is_some() {
            if let Some((name, _)) = buttons::NAMES.iter().find(|(_, b)| {
                input.buttons_pressed & b != 0
                    || input.buttons_down & b != 0 && self.previous_down & b == 0
            }) {
                self.capture(Binding::Key {
                    key: (*name).into(),
                });
            }
        }
    }
    pub fn end_frame(&mut self, input: InputFrame) {
        self.previous_values = self
            .actions
            .iter()
            .map(|(n, s)| (n.clone(), self.evaluate(s)))
            .collect();
        self.previous_down = input.buttons_down;
        self.previous_pointer = (input.pointer_x, input.pointer_y);
        self.suppress_edges.clear();
        self.pressed_buttons.clear();
    }
    pub fn inject_gamepad(&mut self, event: GamepadEvent) -> Result<()> {
        match event {
            GamepadEvent::Connected { id, family } => {
                ensure!(
                    self.pads.contains_key(&id) || self.pads.len() < 16,
                    "too many gamepads"
                );
                self.pads.insert(
                    id,
                    PadState {
                        family,
                        ..Default::default()
                    },
                );
                if self.active_pad.is_none() {
                    self.active_pad = Some(id);
                }
            }
            GamepadEvent::Disconnected { id } => {
                self.pads.remove(&id);
                if self.active_pad == Some(id) {
                    // A different pad's already-held buttons must not become
                    // fresh actions when the active device disconnects.
                    self.active_pad = None;
                    self.gamepad_prompts = false;
                    self.pending_pressed_buttons.clear();
                    self.pressed_buttons.clear();
                    self.navigation_presses = 0;
                }
                if self.pads.is_empty() {
                    self.gamepad_prompts = false;
                }
            }
            GamepadEvent::Button { id, button, value } => {
                ensure!(
                    value.is_finite() && (0. ..=1.).contains(&value),
                    "gamepad button must be finite 0..1"
                );
                let pad = self
                    .pads
                    .get_mut(&id)
                    .context("gamepad must connect before input")?;
                let old = pad.buttons.insert(button, value).unwrap_or(0.);
                if value > 0.1 && (value - old).abs() > 0.01 {
                    self.activate_pad(id);
                    self.gamepad_prompts = true;
                    self.pending_gamepad_activity = true;
                }
                if value > old && self.active_pad == Some(id) {
                    self.pending_pressed_buttons
                        .entry(button)
                        .and_modify(|range| {
                            range.0 = range.0.min(old);
                            range.1 = range.1.max(value);
                        })
                        .or_insert((old, value));
                }
                if old < 0.5 && value >= 0.5 {
                    self.navigation_presses |= ui_bit(button);
                    self.capture(Binding::Button { button });
                }
            }
            GamepadEvent::Axis { id, axis, value } => {
                ensure!(
                    value.is_finite() && (-1. ..=1.).contains(&value),
                    "gamepad axis must be finite -1..1"
                );
                let pad = self
                    .pads
                    .get_mut(&id)
                    .context("gamepad must connect before input")?;
                let old = pad.axes.insert(axis, value).unwrap_or(0.);
                if value.abs() > 0.2 && (value - old).abs() > 0.02 {
                    self.activate_pad(id);
                    self.gamepad_prompts = true;
                    self.pending_gamepad_activity = true;
                }
                if old.abs() < 0.65 && value.abs() >= 0.65 {
                    self.capture(Binding::Axis {
                        axis,
                        scale: 1.,
                        dead_zone: deadzone(),
                    });
                }
            }
        }
        Ok(())
    }
    fn activate_pad(&mut self, id: u32) {
        if self.active_pad != Some(id) {
            self.active_pad = Some(id);
            self.pending_pressed_buttons.clear();
            self.pressed_buttons.clear();
            self.navigation_presses = 0;
        }
    }
    fn capture(&mut self, binding: Binding) {
        if let Some(action) = self.pending_rebind.take() {
            // Only previously validated, bound actions can enter capture mode.
            let spec = ActionBinding {
                bindings: vec![binding.clone()],
                threshold: self.actions[&action].threshold,
            };
            self.rebind(&action, spec)
                .expect("validated captured binding");
            self.rebind_result = Some(RebindResult { action, binding });
        }
    }
    pub fn start_rebind(&mut self, action: &str) -> Result<()> {
        self.binding(action)?;
        self.pending_rebind = Some(action.into());
        self.rebind_result = None;
        Ok(())
    }
    pub fn cancel_rebind(&mut self) {
        self.pending_rebind = None;
    }
    pub fn take_rebind(&mut self) -> Option<RebindResult> {
        self.rebind_result.take()
    }
    pub fn reset_devices(&mut self) {
        for pad in self.pads.values_mut() {
            pad.buttons.clear();
            pad.axes.clear();
        }
        self.frame = InputFrame::default();
        self.previous_values.clear();
        self.previous_down = 0;
        self.pending_pressed_buttons.clear();
        self.pressed_buttons.clear();
        self.navigation_previous = 0;
        self.navigation_presses = 0;
    }
    fn evaluate(&self, spec: &ActionBinding) -> f32 {
        let pad = self.active_pad.and_then(|id| self.pads.get(&id));
        spec.bindings
            .iter()
            .map(|b| match b {
                Binding::Key { key } => {
                    if (self.frame.buttons_down | self.frame.buttons_pressed)
                        & key_bit(key).unwrap_or(0)
                        != 0
                    {
                        1.
                    } else {
                        0.
                    }
                }
                Binding::Button { button } => pad
                    .and_then(|p| p.buttons.get(button))
                    .copied()
                    .unwrap_or(0.),
                Binding::Axis {
                    axis,
                    scale,
                    dead_zone,
                } => {
                    let v = pad.and_then(|p| p.axes.get(axis)).copied().unwrap_or(0.);
                    if v.abs() <= *dead_zone {
                        0.
                    } else {
                        v.signum() * (v.abs() - dead_zone) / (1. - dead_zone) * scale
                    }
                }
            })
            .sum::<f32>()
            .clamp(-1., 1.)
    }
    pub fn value(&self, action: &str) -> Result<f32> {
        Ok(self.evaluate(self.binding(action)?))
    }
    pub fn is_down(&self, action: &str) -> Result<bool> {
        Ok(self.value(action)?.abs() >= self.binding(action)?.threshold)
    }
    pub fn pressed(&self, action: &str) -> Result<bool> {
        let spec = self.binding(action)?;
        if self.suppress_edges.contains(action) {
            return Ok(false);
        }
        let physical = spec.bindings.iter().any(|b| match b {
            Binding::Key { key } => self.frame.buttons_pressed & key_bit(key).unwrap_or(0) != 0,
            Binding::Button { button } => self
                .pressed_buttons
                .get(button)
                .is_some_and(|(old, value)| *old < spec.threshold && *value >= spec.threshold),
            _ => false,
        });
        Ok(physical
            || self.evaluate(spec).abs() >= spec.threshold
                && self
                    .previous_values
                    .get(action)
                    .copied()
                    .unwrap_or(0.)
                    .abs()
                    < spec.threshold)
    }
    pub fn released(&self, action: &str) -> Result<bool> {
        let spec = self.binding(action)?;
        Ok(!self.suppress_edges.contains(action)
            && self.evaluate(spec).abs() < spec.threshold
            && self
                .previous_values
                .get(action)
                .copied()
                .unwrap_or(0.)
                .abs()
                >= spec.threshold)
    }
    pub fn family(&self) -> GamepadFamily {
        self.prompt_family.unwrap_or_else(|| {
            self.active_pad
                .and_then(|id| self.pads.get(&id))
                .map(|p| p.family)
                .unwrap_or_default()
        })
    }
    pub fn set_prompt_family(&mut self, family: Option<GamepadFamily>) {
        self.prompt_family = family;
    }
    pub fn last_device(&self) -> &'static str {
        if self.gamepad_prompts {
            "gamepad"
        } else {
            "keyboard_mouse"
        }
    }
    pub fn prompt(&self, action: &str) -> Result<Prompt> {
        let spec = self.binding(action)?;
        let binding = spec
            .bindings
            .iter()
            .find(|b| matches!(b, Binding::Key { .. }) != self.gamepad_prompts)
            .unwrap_or(&spec.bindings[0]);
        let family = self.family();
        let (device, control, label) = match binding {
            Binding::Key { key } => (
                "keyboard_mouse",
                key.clone(),
                match key.as_str() {
                    "mouse_left" => "LMB".into(),
                    "mouse_right" => "RMB".into(),
                    _ => key.to_uppercase(),
                },
            ),
            Binding::Button { button } => (
                "gamepad",
                token(*button),
                button_label(*button, family).into(),
            ),
            Binding::Axis { axis, .. } => ("gamepad", token(*axis), token(*axis).replace('_', " ")),
        };
        let icon = if device == "gamepad" {
            format!("input/{}/{control}", token(family))
        } else {
            format!("input/keyboard/{control}")
        };
        Ok(Prompt {
            device: device.into(),
            family,
            control,
            label,
            icon,
        })
    }
    /// Default UI/VM navigation, separate from configurable gameplay actions.
    /// Call once per host frame and OR these bits into the runtime InputFrame.
    pub fn navigation_buttons(&mut self) -> (u64, u64) {
        let mut down = 0;
        if let Some(pad) = self.active_pad.and_then(|id| self.pads.get(&id)) {
            for (button, bit) in [
                (PadButton::South, buttons::ENTER),
                (PadButton::DpadUp, buttons::UP),
                (PadButton::DpadDown, buttons::DOWN),
                (PadButton::DpadLeft, buttons::LEFT),
                (PadButton::DpadRight, buttons::RIGHT),
            ] {
                if pad.buttons.get(&button).copied().unwrap_or(0.) >= 0.5 {
                    down |= bit;
                }
            }
            for (axis, negative, positive) in [
                (PadAxis::DpadX, buttons::LEFT, buttons::RIGHT),
                (PadAxis::DpadY, buttons::DOWN, buttons::UP),
            ] {
                let value = pad.axes.get(&axis).copied().unwrap_or(0.);
                if value < -0.5 {
                    down |= negative;
                } else if value > 0.5 {
                    down |= positive;
                }
            }
        }
        let pressed =
            (down & !self.navigation_previous) | std::mem::take(&mut self.navigation_presses);
        self.navigation_previous = down;
        (down, pressed)
    }
}
fn ui_bit(button: PadButton) -> u64 {
    match button {
        PadButton::South => buttons::ENTER,
        PadButton::DpadUp => buttons::UP,
        PadButton::DpadDown => buttons::DOWN,
        PadButton::DpadLeft => buttons::LEFT,
        PadButton::DpadRight => buttons::RIGHT,
        _ => 0,
    }
}
fn button_label(button: PadButton, family: GamepadFamily) -> &'static str {
    use PadButton::*;
    match (family, button) {
        (GamepadFamily::Xbox, South) => "A",
        (GamepadFamily::Xbox, East) => "B",
        (GamepadFamily::Xbox, West) => "X",
        (GamepadFamily::Xbox, North) => "Y",
        (GamepadFamily::Nintendo, South) => "B",
        (GamepadFamily::Nintendo, East) => "A",
        (GamepadFamily::Nintendo, West) => "Y",
        (GamepadFamily::Nintendo, North) => "X",
        (GamepadFamily::Playstation, South) => "Cross",
        (GamepadFamily::Playstation, East) => "Circle",
        (GamepadFamily::Playstation, West) => "Square",
        (GamepadFamily::Playstation, North) => "Triangle",
        (_, South) => "South",
        (_, East) => "East",
        (_, West) => "West",
        (_, North) => "North",
        (_, LeftShoulder) => "LB/L1",
        (_, RightShoulder) => "RB/R1",
        (_, LeftTrigger) => "LT/L2",
        (_, RightTrigger) => "RT/R2",
        (_, Select) => "Select",
        (_, Start) => "Start",
        (_, Guide) => "Guide",
        (_, LeftStick) => "L3",
        (_, RightStick) => "R3",
        (_, DpadUp) => "D-pad Up",
        (_, DpadDown) => "D-pad Down",
        (_, DpadLeft) => "D-pad Left",
        (_, DpadRight) => "D-pad Right",
    }
}

pub struct InputClass;
fn method(
    name: &'static str,
    params: Vec<(&'static str, T)>,
    returns: T,
    doc: &'static str,
) -> MethodInfo {
    MethodInfo {
        name,
        params,
        returns,
        doc,
    }
}
fn info() -> &'static ClassInfo {
    static I: OnceLock<ClassInfo> = OnceLock::new();
    I.get_or_init(||ClassInfo{name:"Input",doc:"Session-scoped keyboard/mouse/gamepad actions with runtime rebinding and semantic prompt tokens.",properties:vec![
        PropertyInfo{name:"pointer",ty:T::List(Box::new(T::Int)),writable:false,doc:"Canvas pointer position."},PropertyInfo{name:"pointerDelta",ty:T::List(Box::new(T::Int)),writable:false,doc:"Pointer delta this frame."},PropertyInfo{name:"keys",ty:T::List(Box::new(T::String)),writable:false,doc:"Supported physical key names."},PropertyInfo{name:"lastDevice",ty:T::String,writable:false,doc:"keyboard_mouse or gamepad, based on meaningful activity."},PropertyInfo{name:"pendingRebind",ty:T::Optional(Box::new(T::String)),writable:false,doc:"Action waiting for a new physical binding."},
    ],methods:vec![
        method("keyboardLocks",vec![],T::Map,"This client's logical Caps/Num/Scroll lock state and revision. Physical state is null unless imported; logical control never toggles the OS keyboard. Session state resets only on successful Runtime restore."),
        method("setKeyboardLocks",vec![("state",T::Map)],T::Map,"Patch this client's logical caps_lock/num_lock/scroll_lock. mode must be logical; physical requests fail explicitly. Returns owner-committed state/revision, not device success."),
        method("enableEvents",vec![("capacity",T::Int)],T::Void,"Enable/replace this client's opt-in logical input journal (1..4096 samples). Accepted App frames feed it once. Disabled by default; action queries remain independent. Overflow is visible, with no silent edge drop."),
        method("disableEvents",vec![],T::Void,"Release this client's journal. Old receipts cannot affect a newly enabled journal."),
        method("events",vec![("maximum",T::Int)],T::Map,"Publish an ordered prefix {samples:[InputSample],receipt:InputReceipt|null}. Reads are idempotent until acknowledgement; leased samples cannot be rewritten by pointer-motion coalescing. Journal JSON u64 values are exact decimal strings (including masks, sequences and timestamps); echo receipts directly. Records model framed digital buttons/pointer, not raw OS/text/gamepad-axis events."),
        method("ackEvents",vec![("receipt",T::Map)],T::Void,"Acknowledge only a published prefix for this client and epoch. Stale/foreign receipts fail without consuming input."),
        method("eventStats",vec![],T::Map,"enabled,client,epoch,pending,capacity. Does not publish or consume samples. Queues are scoped independently of action bindings."),
        method("captureEvents",vec![],T::Map,"Capture portable InputBufferSnapshot logical intents. Contains no device handles. Existing Runtime restore disables public journals; restoreEvents explicitly restores captured data."),
        method("restoreEvents",vec![("snapshot",T::Map)],T::Void,"Validate and atomically replace a logical journal under a fresh epoch. Snapshot observation time must not exceed the current owner clock. Never restores hardware handles or action-map device state."),
        method("bind",vec![("action",T::String),("keys",T::List(Box::new(T::String)))],T::Void,"Replace keyboard bindings."),
        method("rebind",vec![("action",T::String),("spec",T::Map)],T::Void,"Replace {bindings:[{kind:'key'|'button'|'axis',...}],threshold?:0.5}. Validated atomically."),
        method("bindings",vec![("action",T::String)],T::Map,"Read action bindings."),
        method("isDown",vec![("action",T::String)],T::Bool,"Action magnitude exceeds threshold."),method("pressed",vec![("action",T::String)],T::Bool,"Action crossed threshold or bound button pressed this frame."),method("released",vec![("action",T::String)],T::Bool,"Action released this frame."),
        method("value",vec![("action",T::String)],T::Number,"Combined analog/digital signed value in -1..1."),
        method("axis",vec![("negative",T::String),("positive",T::String)],T::Number,"Positive action magnitude minus negative action magnitude."),
        method("prompt",vec![("action",T::String)],T::Map,"Preferred binding {device,family,control,label,icon}; icon is a semantic asset token."),
        method("setPromptFamily",vec![("family",T::Optional(Box::new(T::String)))],T::Void,"Override generic/xbox/playstation/nintendo labels; null restores detection."),
        method("exportBindings",vec![],T::Map,"Return schema-1 profile for fs.writeSave."),method("importBindings",vec![("profile",T::Map)],T::Void,"Validate and replace complete logical profile; raw hardware state is not restored."),
        method("startRebind",vec![("action",T::String)],T::Void,"Capture next pressed key/button or large axis motion."),method("cancelRebind",vec![],T::Void,"Cancel pending capture."),method("takeRebind",vec![],T::Optional(Box::new(T::Map)),"Drain the completed capture result."),
    ]})
}
fn arg<'a>(args: &'a [Value], index: usize, name: &str) -> Result<&'a Value> {
    args.get(index).with_context(|| format!("missing {name}"))
}
fn json<T: Serialize>(value: T) -> Result<Value> {
    Ok(Value::from_json(serde_json::to_value(value)?))
}
impl Class for InputClass {
    fn info(&self) -> &ClassInfo {
        info()
    }
    fn get(&self, cx: &mut EngineContext, _: ObjectId, p: &str) -> Result<Value> {
        let s = cx.state()?;
        Ok(match p {
            "pointer" => json([s.input.pointer_x, s.input.pointer_y])?,
            "pointerDelta" => json([
                s.input
                    .pointer_x
                    .saturating_sub(s.actions.previous_pointer.0),
                s.input
                    .pointer_y
                    .saturating_sub(s.actions.previous_pointer.1),
            ])?,
            "keys" => json(buttons::NAMES.iter().map(|(n, _)| *n).collect::<Vec<_>>())?,
            "lastDevice" => s.actions.last_device().into(),
            "pendingRebind" => s
                .actions
                .pending_rebind
                .clone()
                .map(Value::from)
                .unwrap_or(Value::Null),
            _ => bail!("unknown Input property {p}"),
        })
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        _: ObjectId,
        method: &str,
        args: &[Value],
    ) -> Result<Value> {
        if let Some(value)=crate::input_events_api::call(cx,method,args)? {return Ok(value);}
        if let Some(value)=crate::input_locks_api::call(cx,method,args)? {return Ok(value);}
        let actions = &mut cx.state()?.actions;
        Ok(match method {
            "bind" => {
                let Value::List(keys) = arg(args, 1, "keys")? else {
                    bail!("keys must be list")
                };
                let keys = keys
                    .iter()
                    .map(|v| v.as_str().map(str::to_owned))
                    .collect::<Result<Vec<_>>>()?;
                actions.bind(arg(args, 0, "action")?.as_str()?, &keys)?;
                Value::Null
            }
            "rebind" => {
                let spec = serde_json::from_value(arg(args, 1, "spec")?.to_json())?;
                actions.rebind(arg(args, 0, "action")?.as_str()?, spec)?;
                Value::Null
            }
            "bindings" => json(actions.binding(arg(args, 0, "action")?.as_str()?)?)?,
            "isDown" => actions.is_down(arg(args, 0, "action")?.as_str()?)?.into(),
            "pressed" => actions.pressed(arg(args, 0, "action")?.as_str()?)?.into(),
            "released" => actions.released(arg(args, 0, "action")?.as_str()?)?.into(),
            "value" => Value::Float(actions.value(arg(args, 0, "action")?.as_str()?)? as f64),
            "axis" => Value::Float(
                (actions.value(arg(args, 1, "positive")?.as_str()?)?.abs()
                    - actions.value(arg(args, 0, "negative")?.as_str()?)?.abs())
                .clamp(-1., 1.) as f64,
            ),
            "prompt" => json(actions.prompt(arg(args, 0, "action")?.as_str()?)?)?,
            "setPromptFamily" => {
                let family = match args.first() {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(serde_json::from_value(serde_json::Value::String(
                        v.as_str()?.into(),
                    ))?),
                };
                actions.set_prompt_family(family);
                Value::Null
            }
            "exportBindings" => json(actions.export())?,
            "importBindings" => {
                actions.import(serde_json::from_value(arg(args, 0, "profile")?.to_json())?)?;
                Value::Null
            }
            "startRebind" => {
                actions.start_rebind(arg(args, 0, "action")?.as_str()?)?;
                Value::Null
            }
            "cancelRebind" => {
                actions.cancel_rebind();
                Value::Null
            }
            "takeRebind" => json(actions.take_rebind())?,
            _ => bail!("unknown Input method {method}"),
        })
    }
}
