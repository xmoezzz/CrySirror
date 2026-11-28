// SPDX-License-Identifier: MPL-2.0
//! The v1 RetouchDialogParam records created by RLD opcode 0x42.
use crate::native_numbers::integer;
use anyhow::{Result, ensure};
use serde::Serialize;

use crate::{profile::Resident, view_definitions};

#[derive(Debug, PartialEq, Serialize)]
pub(super) struct Image {
    resource: i32,
    position: [i32; 2],
    name: Vec<u8>,
}

#[derive(Debug, PartialEq, Serialize)]
pub(super) struct Dialog {
    pub id: i32,
    count: i32,
    options: [i32; 2],
    background: Image,
    selection: Image,
    items: Vec<Image>,
}

impl Dialog {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        // 0x6a860 splits exactly 26 fields; missing FCStrings remain empty.
        let mut fields = [b"".as_slice(); 26];
        for (dst, value) in fields.iter_mut().zip(raw.split(|&b| b == b',')) {
            *dst = value;
        }
        let n = |i: usize| integer(fields[i]);
        let image = |i: usize, name: usize| Image {
            resource: n(i),
            position: [n(i + 1), n(i + 2)],
            name: fields[name].to_vec(),
        };
        let count = n(1);
        // 0x768a0 has three physical item slots. A larger count in
        // defDialog would overrun its local array of parsed FCStrings.
        ensure!(count <= 3, "invalid ExHibit v1 dialog item count {count}");
        Ok(Self {
            id: n(0),
            count,
            options: [n(2), n(3)],
            background: image(4, 7),
            // The FCString pointer is always non-null, even for "". In this
            // overload setSelect (0x767e0 -> 0x76860) clears fields +60/+64
            // and discards the optional numeric settings at CSV indices 11/12.
            selection: image(8, 13),
            items: (0..count)
                .map(|item| {
                    let start = 14 + item as usize * 4;
                    image(start, start + 3)
                })
                .collect(),
        })
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub(super) struct DialogV3 {
    dialog: Dialog,
    default_item: i32,
    selection_parameters: [i32; 2],
}

impl DialogV3 {
    pub fn parse(raw: &[u8], resident: Resident) -> Result<Self> {
        // All three v3 handlers split 26 FCStrings. The count is the
        // low byte, and bits 16..23 specify a one-based default button.
        let mut fields = [b"".as_slice(); 26];
        for (dst, value) in fields.iter_mut().zip(raw.split(|&b| b == b',')) {
            *dst = value;
        }
        let n = |i: usize| crate::native_numbers::integer2011(fields[i]);
        let count = n(1) & 255;
        ensure!(count <= 3, "invalid ExHibit v3 dialog item count {count}");
        let default_item = ((n(1) >> 16) & 255) - 1;
        let default_item = if (0..=2).contains(&default_item) {
            default_item
        } else {
            -1
        };
        let layer = if resident.late() {
            view_definitions::layer_late
        } else {
            view_definitions::layer2011
        };
        let image = |i: usize, name: usize| Image {
            resource: n(i),
            position: [n(i + 1), n(i + 2)],
            name: fields[name].to_vec(),
        };
        let mut background = image(4, 7);
        background.resource = layer(background.resource)?;
        let mut selection = image(8, 13);
        selection.resource = layer(selection.resource)?;
        let selection_parameters =
            if selection.name.is_empty() || selection.name == b"*" || n(13) == -1 {
                selection.name.clear();
                [n(11), n(12)]
            } else {
                [0; 2]
            };
        Ok(Self {
            dialog: Dialog {
                id: n(0),
                count,
                options: [n(2), n(3)],
                background,
                selection,
                items: (0..count)
                    .map(|item| {
                        let start = 14 + item as usize * 4;
                        image(start, start + 3)
                    })
                    .collect(),
            },
            default_item,
            selection_parameters,
        })
    }
}
