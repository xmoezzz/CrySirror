// SPDX-License-Identifier: MPL-2.0
//! RLD v3 blockEntry, storeScene and saveViewClip.
use crate::{
    midori_actor, midori_background, midori_message, midori_positions, midori_prelude,
    midori_sound_capture::SoundCapture,
    midori_state,
    profile::Resident,
    registers::{Registers, Value},
};
use anyhow::{Context, Result, ensure};
use std::fmt::Write;

pub(super) const DEFAULT_MARKER: &[u8] =
    b"0;0,0,0,10,res\\g\\gn\\hit.gyu,A...B...C...D...E...F...G...H...I...";

pub(super) struct Scene<'a> {
    pub resident: Resident,
    pub canvas: [u32; 2],
    pub actors: &'a midori_actor::Slots,
    pub background: &'a midori_background::Background,
    pub printer: &'a midori_message::Catalog,
    pub marker: &'a midori_prelude::Marker,
    pub positions: &'a midori_positions::Work,
    pub entries: &'a midori_state::EntryNames,
    pub sound: SoundCapture,
}

pub(super) struct Capture {
    pub before_flush: [(i32, i32); 2],
    pub after_flush: Vec<(i32, i32)>,
    pub texts: Vec<(i32, Vec<u8>)>,
}

impl Scene<'_> {
    pub fn capture(self, regs: &Registers, block_flag: bool, viewport: bool) -> Result<Capture> {
        let save_layers = regs.read(Value::SystemFlag(5))? != 0;
        let printer_open = self
            .printer
            .checkpoint_open()
            .context("ExHibit printer is changing")?;
        ensure!(!self.marker.preparing(), "ExHibit marker is changing");
        let background = self
            .background
            .checkpoint_draw()
            .context("ExHibit background is changing")?;
        let rows = self
            .actors
            .checkpoint_rows()
            .context("ExHibit portrait is changing")?;
        if save_layers {
            // 2011 saveLayer visits logical IDs 51–54 (native 131–134).
            // Inactive actor definitions allocate no layer. An installed
            // printer may still own its planes while the frame is hidden.
            ensure!(
                self.resident == Resident::Midori2011
                    && self.printer.installed().is_none()
                    && !rows.iter().any(|row| {
                        row[3].is_some_and(|active| active >= 0)
                            && row[1..3]
                                .iter()
                                .flatten()
                                .any(|id| (131..=134).contains(id))
                    }),
                "ExHibit v3 checkpoint needs serialized live layers"
            );
        }
        let mut draw_slots = String::from("8");
        for row in rows {
            let inactive = row[3] == Some(-1);
            draw_slots.push(';');
            for (column, word) in row.into_iter().enumerate() {
                let value = if inactive && matches!(column, 8 | 9) {
                    // The native constructor leaves these two raw coordinates unwritten.
                    // An inactive row never places a sprite, so use a stable neutral value.
                    word.unwrap_or(0)
                } else {
                    word.context("missing ExHibit portrait checkpoint field")?
                };
                if column != 0 {
                    draw_slots.push(',');
                }
                write!(&mut draw_slots, "{value}").expect("String writer");
            }
        }

        let socket = self.printer.checkpoint_socket();
        let socket_index = socket.as_ref().and_then(|v| v.index).map_or(-1, |v| v.1);
        let socket_packet = socket
            .and_then(|v| v.packet)
            .map_or_else(Vec::new, |v| v.1.as_bytes().to_vec());
        // There is no entered-character, local toolbar or viewport/camera
        // mutation in the admitted 2011 instruction surface. Draw portraits
        // are separate and use T205 even when the entered list is empty.
        let flags = (regs.read_normal(4)? & !0x83)
            | 2
            | i32::from(printer_open)
            | if block_flag { 0x80 } else { 0 };
        let mut after_flush: Vec<_> = (10..29).zip(background).collect();
        after_flush.extend((100..148).map(|slot| (slot, -1)));
        after_flush.push((self.sound.normal.0 as i32, self.sound.normal.1));
        let late = self.resident.late();
        if late {
            // The later draw descriptor also writes its untouched
            // extension word at graphic-register base 77 + 4.
            after_flush.push((81, 0));
        }
        if viewport {
            let [width, height] = self.canvas;
            after_flush.extend([(7, 0), (8, ((width << 16) | height) as i32), (9, 256)]);
        }
        let marker = self
            .marker
            .capture()
            .map_or_else(|| DEFAULT_MARKER.to_vec(), |v| v.1.into_bytes());
        let position = self.positions.capture();
        let filter = self.actors.capture_filter(regs.text(4)?);
        let mut texts = vec![
            (3, Vec::new()),
            (5, b"-1;8;0,0,0,0;-1;;;;;;;;".to_vec()),
            (6, socket_packet),
            (7, position.1.into_bytes()),
            (
                8,
                if late {
                    b"3;0;0".to_vec()
                } else {
                    b"1;0".to_vec()
                },
            ),
            (201, Vec::new()),
            (202, marker),
            (203, Vec::new()),
            (self.sound.text.0 as i32, self.sound.text.1.into_bytes()),
            (205, draw_slots.into_bytes()),
            (207, self.entries.checkpoint_text()),
        ];
        if late {
            // The constructor's three strings are empty. Its reader only
            // uses the mode when a source is supplied (0x268330).
            texts.push((210, b"0,,,".to_vec()));
        }
        if save_layers {
            // No layer passed the native validity test at 0xf4930.
            texts.push((208, Vec::new()));
        }
        if let Some((index, packet)) = filter {
            texts.push((index as i32, packet.into_bytes()));
        }
        Ok(Capture {
            before_flush: [(4, flags), (78, socket_index)],
            after_flush,
            texts,
        })
    }
}

pub(super) fn title(regs: &Registers, raw: &[u8], block: i32) -> Result<Vec<u8>> {
    let raw = if raw.is_empty() || raw == b"*" {
        regs.text(206)?
    } else {
        raw
    };
    let path = regs.text(2)?;
    let block = block.to_string();
    let mut result = Vec::with_capacity(raw.len());
    let mut bytes = raw.iter().copied().peekable();
    while let Some(byte) = bytes.next() {
        match (byte, bytes.peek()) {
            (b'$', Some(b's')) => {
                bytes.next();
                result.extend_from_slice(path);
            }
            (b'$', Some(b'b')) => {
                bytes.next();
                result.extend_from_slice(block.as_bytes());
            }
            _ => result.push(byte),
        }
    }
    Ok(result)
}
