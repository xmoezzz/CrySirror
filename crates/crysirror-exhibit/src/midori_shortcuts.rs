// SPDX-License-Identifier: MPL-2.0
//! Midori 2011 authored shortcut bindings; physical events and actions are
//! separate owners and are not executed by these definition records.
use crate::branch::number;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::collections::BTreeMap;

const KEY_CODES: [u8; 17] = [
    27, 112, 113, 114, 115, 116, 117, 118, 119, 120, 8, 33, 34, 2, 121, 122, 123,
];

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(super) struct Binding {
    key: u8,
    action: i32,
    allowed: u8,
    args: [i32; 4],
    parameter: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Define {
    clear: bool,
    binding: Binding,
}

impl Define {
    pub fn parse(flags: u8, raw: &[u8]) -> Result<Self> {
        let mut fields = raw.splitn(8, |&b| b == b',');
        let mut word = || number(fields.next().context("missing Midori shortcut operand")?);
        let id = usize::try_from(word()?)?;
        let key = if id < 32 {
            *KEY_CODES
                .get(id)
                .context("unknown Midori shortcut key ID")?
        } else {
            u8::try_from(id)?
        };
        let action = word()?;
        let mask = u8::try_from(word()?)?;
        ensure!(
            mask & !0x87 == 0 && (0..=4096).contains(&action),
            "unsupported Midori shortcut mask or action"
        );
        let args = [word()?, word()?, word()?, word()?];
        let parameter = match (flags, fields.next()) {
            (0x11, None) => None,
            (0x21, Some(packet)) if key != 2 && action == 104 => {
                // cmdDefKey1 converts each ';' to ',' while retaining other
                // bytes; keep this packet for the action owner, don't run it.
                ensure!(
                    packet.len() <= 1024
                        && !packet.is_empty()
                        && packet.contains(&b';')
                        && packet.iter().all(|b| b.is_ascii_hexdigit()
                            || matches!(b, b'x' | b'X' | b' ' | b'\t' | b';' | b'+' | b'-' | b'*')),
                    "unsupported Midori shortcut action packet"
                );
                Some(std::str::from_utf8(packet)?.replace(';', ","))
            }
            _ => anyhow::bail!("unsupported Midori shortcut operand form"),
        };
        Ok(Self {
            clear: mask & 0x80 != 0,
            binding: Binding {
                key,
                action,
                allowed: mask & 7,
                args,
                parameter,
            },
        })
    }
}

#[derive(Default, Serialize)]
pub(super) struct Catalog {
    keys: BTreeMap<u8, Binding>,
    right: Option<Binding>,
    legacy_key_slots: Option<[i32; 4]>,
}
impl Catalog {
    pub fn define(&mut self, definition: Define) {
        if definition.clear {
            self.keys.clear();
            self.legacy_key_slots = Some([-1; 4]);
        }
        if definition.binding.key == 2 {
            self.right = Some(definition.binding);
        } else {
            self.keys.insert(definition.binding.key, definition.binding);
        }
    }

    #[cfg(test)]
    pub fn ordinary_count(&self) -> usize {
        self.keys.len()
    }
}
