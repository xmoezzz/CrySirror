// SPDX-License-Identifier: MPL-2.0
//! UxAdvSystem::setSkipBlockMode and UxAsyncSubsystem::asyncProc.
use crysirror_core::service::{InputFrame, buttons};

#[derive(Default)]
pub(super) enum SkipBlock {
    #[default]
    Outside,
    WaitingForRelease,
    Armed,
    Active,
}

impl SkipBlock {
    pub(super) fn set(&mut self, enter: bool) {
        *self = if enter {
            Self::WaitingForRelease
        } else {
            Self::Outside
        };
    }

    pub(super) fn poll(&mut self, input: InputFrame, canvas: [u32; 2]) {
        // WindowFromPoint must belong to the game's root window. Canvas
        // coordinates outside the host game surface cannot arm or trigger it.
        if !(0..i64::from(canvas[0])).contains(&i64::from(input.pointer_x))
            || !(0..i64::from(canvas[1])).contains(&i64::from(input.pointer_y))
        {
            return;
        }
        let down = input.buttons_down
            & (buttons::CTRL | buttons::MOUSE_LEFT | buttons::SPACE | buttons::ENTER)
            != 0;
        match self {
            Self::WaitingForRelease if !down => *self = Self::Armed,
            Self::Armed if down => *self = Self::Active,
            _ => {}
        }
    }

    pub(super) fn active(&self) -> bool {
        matches!(self, Self::Active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Controls, Script};

    #[test]
    fn skip_block_requires_release_then_new_press_and_only_cancels_wait_inside_block()
    -> anyhow::Result<()> {
        let mut bytes = vec![0; 20];
        bytes[..4].copy_from_slice(b"\0DLR");
        bytes[4..8].copy_from_slice(&3u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&16u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&2u32.to_le_bytes());
        // Both native commands consume a decoded CMD_DATA and ignore its fields.
        bytes.extend(0x1101_00f9u32.to_le_bytes());
        bytes.extend(99u32.to_le_bytes());
        bytes.extend(b"ignored\0");
        bytes.extend(0xfau32.to_le_bytes());
        for button in [
            buttons::CTRL,
            buttons::MOUSE_LEFT,
            buttons::SPACE,
            buttons::ENTER,
        ] {
            let mut script = Script::new(bytes.clone(), 3)?;
            let mut controls = Controls::default();
            let Action::SkipBlock(enter) = script.next()? else {
                anyhow::bail!("missing enter")
            };
            controls.block.set(enter);
            let held = InputFrame {
                buttons_down: button,
                ..Default::default()
            };
            let size = [1280, 720];
            controls.block.poll(held, size);
            controls.block.poll(
                InputFrame {
                    pointer_x: -1,
                    ..Default::default()
                },
                size,
            );
            controls.block.poll(held, size);
            assert_eq!(controls.delay(250), 250);
            controls.block.poll(InputFrame::default(), size);
            controls.block.poll(
                InputFrame {
                    pointer_y: 720,
                    ..held
                },
                size,
            );
            assert_eq!(controls.delay(250), 250);
            controls.block.poll(held, size);
            controls.block.poll(InputFrame::default(), size);
            assert_eq!(controls.delay(250), 0);
            let Action::SkipBlock(enter) = script.next()? else {
                anyhow::bail!("missing leave")
            };
            controls.block.set(enter);
            assert_eq!(controls.delay(250), 250);
            assert_eq!(script.remaining, 0);
        }
        Ok(())
    }
}
