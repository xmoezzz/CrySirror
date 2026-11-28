// SPDX-License-Identifier: MPL-2.0
//! Retained UxLocalCtrl definitions (cmdDefCtrl2). This path owns the
//! definitions before the resident creates visible controls.
use crate::{native_numbers::integer2011, profile::Resident, view_definitions};
use anyhow::{Result, ensure};
use serde::Serialize;

#[derive(Debug, PartialEq, Eq, Serialize)]
struct Image {
    name: Vec<u8>,
    origin: [i32; 2],
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct ActionPacket {
    mode: i32,
    words: [i32; 4],
    payload: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct MenuItem {
    id: i32,
    image: Image,
    words: [i32; 2],
    action: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(super) struct Definition {
    pub slot: u8,
    action_id: i32,
    layer: i32,
    flags: u32,
    background: Option<Image>,
    atlas: Option<Image>,
    atlas_options: [i32; 3],
    button: ActionPacket,
    alternate: ActionPacket,
    information: Option<(i32, Image)>,
    menu_layer: i32,
    menu_backgrounds: [Option<Image>; 2],
    menu_label: Vec<u8>,
    menu_keys: [i32; 2],
    items: Vec<Option<MenuItem>>,
}

fn key_code(raw: i32) -> i32 {
    const KEYS: [i32; 17] = [
        27, 112, 113, 114, 115, 116, 117, 118, 119, 120, 8, 33, 34, 2, 121, 122, 123,
    ];
    match raw {
        0..=16 => KEYS[raw as usize],
        32..=255 => raw,
        _ => -1,
    }
}

fn semicolons(raw: &[u8]) -> Vec<u8> {
    raw.iter()
        .map(|&b| if b == b';' { b',' } else { b })
        .collect()
}

fn menu_action(raw: &[u8]) -> Vec<u8> {
    // '@' can be the second byte of a CP932 character. FCString's split
    // treats only the single-byte character as a separator.
    let mut at = 0;
    while at < raw.len() {
        let byte = raw[at];
        if byte == b'@' {
            let mut output = semicolons(&raw[..at]);
            let rest = raw[at + 1..].trim_ascii_start();
            if !rest.is_empty() {
                output.push(b'@');
                output.extend_from_slice(rest);
            }
            return output;
        }
        at += if matches!(byte, 0x81..=0x9f | 0xe0..=0xfc) {
            2
        } else {
            1
        };
    }
    semicolons(raw)
}

impl Definition {
    pub fn parse(resident: Resident, flags: u8, raw: &[u8]) -> Result<Option<Self>> {
        let late = resident.late();
        let options_count = if late { 9 } else { 7 };
        let mode = flags >> 4;
        let prefix = match mode {
            0 => 24,
            1 => 25,
            2 => 33,
            3 => 37,
            4 => 38,
            _ => 42,
        };
        let count = if late && mode <= 5 {
            // The legacy layouts in the late parser still split only the
            // first seven entries. Its last two item constructors see empty fields.
            prefix + 7 * 7
        } else {
            prefix + 7 * options_count
        };
        let mut fields = [b"".as_slice(); 105];
        for (dest, field) in fields.iter_mut().take(count).zip(raw.split(|&b| b == b',')) {
            *dest = field;
        }
        let number = |i: usize| integer2011(fields[i]);
        let slot = number(0) as u32;
        if slot >= 16 {
            return Ok(None);
        }
        let layer_alias = if late {
            view_definitions::layer_late
        } else {
            view_definitions::layer2011
        };
        let layer = layer_alias(number(2))?;
        let image = |i: usize| match fields[i] {
            b"" | b"-1" => None,
            name => Some(Image {
                name: name.to_vec(),
                origin: [number(i + 1), number(i + 2)],
            }),
        };
        let mut at = 13;
        let packet = |at: &mut usize, mode_word| {
            let mut words = [0; 4];
            if mode >= 2 {
                words = std::array::from_fn(|j| number(*at + j));
                *at += 4;
            }
            let mut payload = Vec::new();
            if mode >= 5 {
                let raw = fields[*at];
                if !matches!(raw, b"" | b"*") && !(late && raw == b"NULL") {
                    payload = if late { semicolons(raw) } else { raw.to_vec() };
                }
                *at += 2;
            }
            if late && mode_word == 254 && words[0] == 1 && words[1] == -1 {
                words[1] = slot as i32;
            }
            ActionPacket {
                mode: mode_word,
                words,
                payload,
            }
        };
        let button = packet(&mut at, number(12));
        let alternate_mode = number(at);
        // Both packets have the same layout; the second starts just after
        // its mode word. Its fields precede the optional information image.
        at += 1;
        let alternate = packet(&mut at, alternate_mode);
        let information = if mode >= 3 {
            let native_layer = number(at);
            let origin = [number(at + 1), number(at + 2)];
            let name = fields[at + 3];
            at += 4;
            let mut info_layer = if late {
                layer_alias(native_layer)?
            } else {
                native_layer
            };
            if info_layer == -1 {
                info_layer = if late { 185 } else { 172 };
            }
            (info_layer > 0).then(|| {
                (
                    info_layer,
                    Image {
                        name: name.to_vec(),
                        origin,
                    },
                )
            })
        } else {
            None
        };
        if mode >= 4 {
            at += 1; // Duration for immediate instantiation.
        }
        let control_flags = if mode == 0 {
            u32::from((1..=3).contains(&slot))
        } else {
            number(at + 10) as u32
        };
        ensure!(
            control_flags & 8 == 0,
            "unsupported ExHibit immediate local-control instantiation"
        );
        ensure!(
            number(9) != 2,
            "unsupported ExHibit composite local-control atlas"
        );
        let item_from = |i: usize| {
            let id = number(i);
            if id < 0 || (late && fields[i].is_empty()) {
                return None;
            }
            Some(MenuItem {
                id,
                image: Image {
                    name: fields[i + 1].to_vec(),
                    origin: [number(i + 2), number(i + 3)],
                },
                words: [number(i + 4), number(i + 5)],
                action: if fields[i + 6] == b"*" {
                    Vec::new()
                } else {
                    menu_action(fields[i + 6])
                },
            })
        };
        let action_id = number(1);
        let menu_layer = number(at);
        Ok(Some(Self {
            slot: slot as u8,
            action_id: if action_id == -1 {
                slot as i32 + 210
            } else {
                action_id
            },
            layer: if layer == -1 {
                slot as i32 + if late { 160 } else { 153 }
            } else {
                layer
            },
            flags: control_flags,
            background: image(3),
            atlas: image(6),
            atlas_options: [number(9), number(10), number(11)],
            button,
            alternate,
            information,
            menu_layer: if menu_layer == -1 {
                if late { 182 } else { 169 }
            } else {
                menu_layer
            },
            menu_backgrounds: [image(at + 1), image(at + 4)],
            menu_label: match fields[at + 7] {
                b"-1" => Vec::new(),
                raw => raw.to_vec(),
            },
            menu_keys: [key_code(number(at + 8)), key_code(number(at + 9))],
            items: (0..options_count)
                .map(|j| item_from(prefix + 7 * j))
                .collect(),
        }))
    }
}
