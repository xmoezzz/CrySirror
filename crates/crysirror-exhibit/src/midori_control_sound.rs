// SPDX-License-Identifier: MPL-2.0
//! Midori 2011 UxLocalCtrl's 100 system sound bindings, shared by commands
//! 0xbe, 0xea and authored control definitions. Registration schedules no audio.
use crate::native_numbers::{integer2011, unsigned2011};
use serde::Serialize;
use std::collections::BTreeMap;

const COUNT: u8 = 100;
// CP932 "標準音", resident RVA 0x2244b4.
const DEFAULT_LABEL: &[u8] = &[0x95, 0x57, 0x8f, 0x80, 0x89, 0xb9];

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(super) struct Choice {
    id: i32,
    label: Vec<u8>,
}

#[derive(Default, Debug, Serialize)]
pub(super) struct Details {
    flags: u32,
    filename: Vec<u8>,
    choices: Vec<Choice>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Command {
    first: i32,
    ids: Vec<i32>,
    reset: bool,
    filename: Vec<u8>,
    extra: Vec<Choice>,
}

impl Command {
    pub fn basic(raw: &[u8]) -> Self {
        let mut fields = raw.split(|&c| c == b',');
        let slot = integer2011(fields.next().unwrap_or_default());
        let value = fields.next().unwrap_or_default();
        let (first, ids) = if slot == -1 {
            let ids = value
                .split(|&c| c == b'.')
                .filter(|entry| !entry.is_empty())
                .take(usize::from(COUNT))
                .map(integer2011)
                .collect();
            (0, ids)
        } else {
            (slot, vec![integer2011(value)])
        };
        Self {
            first,
            ids,
            reset: false,
            filename: vec![],
            extra: vec![],
        }
    }

    pub fn extended(raw: &[u8]) -> Self {
        let mut fields = raw.splitn(6, |&c| c == b',');
        let first = integer2011(fields.next().unwrap_or_default());
        let id = integer2011(fields.next().unwrap_or_default());
        let flags = unsigned2011(fields.next().unwrap_or_default());
        let filename = fields.next().unwrap_or_default().to_vec();
        // The native command formats "primary,label" before the optional
        // suffix. The label itself can contain semicolon-separated choices.
        let mut list = id.to_string().into_bytes();
        list.push(b',');
        list.extend_from_slice(fields.next().unwrap_or_default());
        if flags & 1 != 0 {
            list.push(b';');
            list.extend_from_slice(fields.next().unwrap_or_default());
        }
        let extra = list
            .split(|&c| c == b';')
            .filter(|row| !row.is_empty())
            .map(|row| {
                let mut fields = row.split(|&c| c == b',');
                Choice {
                    id: integer2011(fields.next().unwrap_or_default()),
                    label: fields.next().unwrap_or_default().to_vec(),
                }
            })
            .collect();
        Self {
            first,
            ids: vec![id],
            reset: flags & 2 != 0,
            filename,
            extra,
        }
    }
}

#[derive(Default)]
pub(super) struct Catalog {
    pub ids: BTreeMap<u8, i32>,
    pub details: BTreeMap<u8, Details>,
}

impl Catalog {
    pub fn plain(&mut self, slot: u8, id: i32) {
        self.assign(slot, id, &[], &[]);
    }

    fn assign(&mut self, slot: u8, id: i32, filename: &[u8], extra: &[Choice]) {
        self.ids.insert(slot, id);
        let detail = self.details.entry(slot).or_default();
        detail.flags &= !1;
        detail.filename.clear();
        detail.filename.extend_from_slice(filename);
        detail.choices.clear();
        detail.choices.push(Choice {
            id,
            label: DEFAULT_LABEL.to_vec(),
        });
        for choice in extra {
            if !detail.choices.iter().any(|other| other.id == choice.id) {
                detail.choices.push(Choice {
                    id: choice.id,
                    label: choice.label.clone(),
                });
            }
        }
    }

    pub fn apply(&mut self, command: Command) {
        if command.reset {
            for slot in 0..COUNT {
                self.ids.insert(slot, -1);
                let detail = self.details.entry(slot).or_default();
                detail.flags = 0;
                detail.choices.clear();
            }
        }
        if let Ok(first) = u8::try_from(command.first) {
            for (slot, id) in (first..COUNT).zip(command.ids) {
                self.assign(slot, id, &command.filename, &command.extra);
            }
        }
    }
}
