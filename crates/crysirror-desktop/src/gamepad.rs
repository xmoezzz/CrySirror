// SPDX-License-Identifier: MPL-2.0
//! Optional native gamepad event source. Owns GilRs, never enters saved data.
//! Compile with desktop feature `gamepad = ["dep:gilrs", "scripting"]`.
use anyhow::{Context, Result};
use crysirror_core::service::InputFrame;
use crysirror_engine::input_api::{ActionMap, GamepadEvent, GamepadFamily, PadAxis, PadButton};
use gilrs::{Axis, Button, EventType, Gilrs};
use std::collections::BTreeSet;

pub struct Backend {
    gilrs: Gilrs,
    pending: Vec<GamepadEvent>,
    announced: BTreeSet<u32>,
    navigation: ActionMap,
}
fn id(value: gilrs::GamepadId) -> Result<u32> {
    u32::try_from(usize::from(value)).context("gamepad ID exceeds engine range")
}
/// Label detection is deliberately a name heuristic, with an explicit engine
/// override. It is not a promise that a third-party pad has a particular layout.
fn family(name: &str) -> GamepadFamily {
    let n = name.to_lowercase();
    if ["dualsense", "dualshock", "playstation", "sony"]
        .iter()
        .any(|s| n.contains(s))
    {
        GamepadFamily::Playstation
    } else if ["xbox", "x-box", "xinput", "microsoft"]
        .iter()
        .any(|s| n.contains(s))
    {
        GamepadFamily::Xbox
    } else if ["nintendo", "joy-con", "switch pro"]
        .iter()
        .any(|s| n.contains(s))
    {
        GamepadFamily::Nintendo
    } else {
        GamepadFamily::Generic
    }
}
fn button(value: Button) -> Option<PadButton> {
    use PadButton as P;
    Some(match value {
        Button::South => P::South,
        Button::East => P::East,
        Button::West => P::West,
        Button::North => P::North,
        Button::LeftTrigger => P::LeftShoulder,
        Button::RightTrigger => P::RightShoulder,
        Button::LeftTrigger2 => P::LeftTrigger,
        Button::RightTrigger2 => P::RightTrigger,
        Button::Select => P::Select,
        Button::Start => P::Start,
        Button::Mode => P::Guide,
        Button::LeftThumb => P::LeftStick,
        Button::RightThumb => P::RightStick,
        Button::DPadUp => P::DpadUp,
        Button::DPadDown => P::DpadDown,
        Button::DPadLeft => P::DpadLeft,
        Button::DPadRight => P::DpadRight,
        _ => return None,
    })
}
fn axis(value: Axis) -> Option<PadAxis> {
    use PadAxis as P;
    Some(match value {
        Axis::LeftStickX => P::LeftX,
        Axis::LeftStickY => P::LeftY,
        Axis::RightStickX => P::RightX,
        Axis::RightStickY => P::RightY,
        Axis::LeftZ => P::LeftTrigger,
        Axis::RightZ => P::RightTrigger,
        Axis::DPadX => P::DpadX,
        Axis::DPadY => P::DpadY,
        _ => return None,
    })
}
impl Backend {
    pub fn new() -> Result<Self> {
        let gilrs = Gilrs::new()
            .map_err(|e| anyhow::anyhow!("initialize optional gamepad backend: {e}"))?;
        let mut pending = Vec::new();
        let mut announced = BTreeSet::new();
        for (gamepad_id, pad) in gilrs.gamepads() {
            let id = id(gamepad_id)?;
            pending.push(GamepadEvent::Connected {
                id,
                family: family(pad.name()),
            });
            announced.insert(id);
        }
        Ok(Self {
            gilrs,
            pending,
            announced,
            navigation: ActionMap::default(),
        })
    }
    /// Poll on the desktop event-loop thread before App::frame. The returned
    /// plain events can also be recorded/replayed or injected in offline tests.
    pub fn poll(&mut self) -> Result<Vec<GamepadEvent>> {
        let mut out = std::mem::take(&mut self.pending);
        for _ in 0..4096 {
            let Some(event) = self.gilrs.next_event() else {
                break;
            };
            let id = id(event.id)?;
            if matches!(event.event, EventType::Connected) {
                if self.announced.insert(id) {
                    out.push(GamepadEvent::Connected {
                        id,
                        family: family(self.gilrs.gamepad(event.id).name()),
                    });
                }
                continue;
            }
            if matches!(event.event, EventType::Disconnected) {
                self.announced.remove(&id);
                out.push(GamepadEvent::Disconnected { id });
                continue;
            }
            if !self.announced.contains(&id) {
                self.announced.insert(id);
                out.push(GamepadEvent::Connected {
                    id,
                    family: family(self.gilrs.gamepad(event.id).name()),
                });
            }
            match event.event {
                EventType::ButtonPressed(b, _) => {
                    if let Some(button) = button(b) {
                        out.push(GamepadEvent::Button {
                            id,
                            button,
                            value: 1.,
                        });
                    }
                }
                EventType::ButtonReleased(b, _) => {
                    if let Some(button) = button(b) {
                        out.push(GamepadEvent::Button {
                            id,
                            button,
                            value: 0.,
                        });
                    }
                }
                EventType::ButtonChanged(b, v, _) => {
                    if let Some(button) = button(b) {
                        out.push(GamepadEvent::Button {
                            id,
                            button,
                            value: v.clamp(0., 1.),
                        });
                    }
                }
                EventType::AxisChanged(a, v, _) => {
                    if let Some(axis) = axis(a) {
                        out.push(GamepadEvent::Axis {
                            id,
                            axis,
                            value: v.clamp(-1., 1.),
                        });
                    }
                }
                _ => (),
            }
        }
        self.gilrs.inc();
        for event in &out {
            self.navigation.inject_gamepad(*event)?;
        }
        Ok(out)
    }
    /// Default south/confirm and D-pad routing for retained UI and native VMs.
    /// Gameplay analog/button actions use the events returned from poll instead.
    pub fn navigation_frame(&mut self, mut input: InputFrame) -> InputFrame {
        let (down, pressed) = self.navigation.navigation_buttons();
        input.buttons_down |= down;
        input.buttons_pressed |= pressed;
        input
    }
    pub fn reset_navigation(&mut self) {
        self.navigation.reset_devices();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn labels_and_native_controls_map_without_a_device() {
        assert_eq!(family("Sony DualSense"), GamepadFamily::Playstation);
        assert_eq!(family("Xbox Wireless"), GamepadFamily::Xbox);
        assert_eq!(family("Nintendo Switch Pro"), GamepadFamily::Nintendo);
        assert_eq!(family("Unidentified USB"), GamepadFamily::Generic);
        assert_eq!(button(Button::LeftTrigger), Some(PadButton::LeftShoulder));
        assert_eq!(button(Button::LeftTrigger2), Some(PadButton::LeftTrigger));
        assert_eq!(axis(Axis::LeftStickX), Some(PadAxis::LeftX));
        assert_eq!(axis(Axis::Unknown), None);
    }
}
