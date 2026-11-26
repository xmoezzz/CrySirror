// SPDX-License-Identifier: MPL-2.0
//! A PS3 pad driven from the keyboard, for this title only. The layout is
//! RPCS3's default keyboard pad (`keyboard_pad_handler::init_config`):
//! start Enter, select Space, square Z, cross X, circle C, triangle V,
//! d-pad on the arrows, R1 E, R2 T, R3 G, L1 Q, L2 R, L3 F, the left stick
//! on W/A/S/D and the right stick on Home (up) / End (down) / Delete (left)
//! / Page Down (right). The host's key mask has no X or G, so cross is on
//! Space, select moves to Tab and R3 is on B.
//!
//! Buttons use the bits scripts see (`sub_137B28`); the keyboard gives
//! full stick deflection.
use crysirror_core::service::{InputFrame, buttons as b};

pub const UP: u32 = 0x1;
pub const LEFT: u32 = 0x2;
pub const RIGHT: u32 = 0x4;
pub const DOWN: u32 = 0x8;
pub const L2: u32 = 0x10;
pub const R2: u32 = 0x20;
pub const CROSS: u32 = 0x40;
pub const L1: u32 = 0x80;
pub const R1: u32 = 0x100;
pub const SQUARE: u32 = 0x200;
pub const TRIANGLE: u32 = 0x400;
pub const CIRCLE: u32 = 0x800;
pub const R3: u32 = 0x1000;
pub const START: u32 = 0x2000;
pub const SELECT: u32 = 0x4000;
pub const L3: u32 = 0x8000;

/// Host key → pad button.
pub const KEYS: [(u64, u32); 16] = [
    (b::ENTER, START),
    (b::TAB, SELECT),
    (b::KEY_Z, SQUARE),
    (b::SPACE, CROSS),
    (b::KEY_C, CIRCLE),
    (b::KEY_V, TRIANGLE),
    (b::LEFT, LEFT),
    (b::DOWN, DOWN),
    (b::RIGHT, RIGHT),
    (b::UP, UP),
    (b::KEY_E, R1),
    (b::KEY_T, R2),
    (b::KEY_B, R3),
    (b::KEY_Q, L1),
    (b::KEY_R, L2),
    (b::KEY_F, L3),
];

/// Pad state for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pad {
    pub held: u32,
    pub pressed: u32,
    pub released: u32,
    /// Sticks: x right, y up, each −1..1.
    pub left: [f32; 2],
    pub right: [f32; 2],
}

impl Pad {
    /// The pad for `input`, with releases against the previous frame.
    pub fn read(previous: &Pad, input: &InputFrame) -> Pad {
        let bits = |mask: u64| KEYS.iter().filter(|(k, _)| mask & k != 0).fold(0, |acc, (_, v)| acc | v);
        let key = |m: u64| f32::from(u8::from(input.buttons_down & m != 0));
        let held = bits(input.buttons_down);
        Pad {
            held,
            pressed: bits(input.buttons_pressed),
            released: previous.held & !held,
            left: [key(b::KEY_D) - key(b::KEY_A), key(b::KEY_W) - key(b::KEY_S)],
            right: [key(b::PAGE_DOWN) - key(b::DELETE), key(b::HOME) - key(b::END)],
        }
    }
    pub fn down(&self, button: u32) -> bool {
        self.held & button != 0
    }
    pub fn hit(&self, button: u32) -> bool {
        self.pressed & button != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_distinct_buttons() {
        let all = KEYS.iter().fold(0u32, |acc, (_, v)| {
            assert_eq!(acc & v, 0);
            acc | v
        });
        assert_eq!(all, 0xffff);
        let input = InputFrame { buttons_down: b::SPACE | b::KEY_W | b::HOME, buttons_pressed: b::SPACE, pointer_x: 0, pointer_y: 0 };
        let pad = Pad::read(&Pad { held: SQUARE, ..Default::default() }, &input);
        assert!(pad.hit(CROSS) && pad.down(CROSS));
        assert_eq!(pad.released, SQUARE);
        assert_eq!((pad.left, pad.right), ([0., 1.], [0., 1.]));
    }
}
