// SPDX-License-Identifier: MPL-2.0
//! Authored control parts and plain system-SE table in the 2011 prelude.
use crate::{branch::number, native_numbers::integer2011};
use anyhow::{Result, ensure};
use crysirror_core::path::{Mount, VfsPath};
use std::collections::BTreeMap;

const SOUND_SLOTS: usize = 100;

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct Notification {
    pub slot: u8,
    position: [i32; 2],
    resource: Vec<u8>,
}

impl Notification {
    pub fn parse(raw: &[u8]) -> Option<Self> {
        // The two later cmdDefNotify handlers feed UxLocalCtrl::defParts mode
        // 3. The unsigned slot check precedes the owned string assignment.
        let mut fields = raw.splitn(4, |&b| b == b',');
        let slot = integer2011(fields.next().unwrap_or_default()) as u32;
        if slot >= 32 {
            return None;
        }
        let position = std::array::from_fn(|_| integer2011(fields.next().unwrap_or_default()));
        Some(Self {
            slot: slot as u8,
            position,
            resource: fields.next().unwrap_or_default().to_vec(),
        })
    }

    pub fn bundle(raw: &[u8]) -> Option<Vec<Self>> {
        let mut fields = raw.split(|&b| b == b',');
        if integer2011(fields.next().unwrap_or_default()) != 3 {
            return None;
        }
        Some(
            (0..8)
                .filter_map(|slot| {
                    let resource = fields.next().unwrap_or_default();
                    let position =
                        std::array::from_fn(|_| integer2011(fields.next().unwrap_or_default()));
                    (integer2011(resource) != -1).then(|| Self {
                        slot,
                        position,
                        resource: resource.to_vec(),
                    })
                })
                .collect(),
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Part {
    pub slot: u8,
    pub position: [i32; 2],
    asset_id: u16,
}

impl Part {
    fn asset(raw: &[u8]) -> Result<u16> {
        Ok(match raw {
            br"res\g\sy\01\01_920.gyu" => 920,
            br"res\g\sy\01\01_921.gyu" => 921,
            br"res\g\sy\01\01_922.gyu" => 922,
            _ => anyhow::bail!("unsupported Midori control part asset"),
        })
    }

    pub fn parse(raw: &[u8]) -> Result<Self> {
        let fields: Vec<_> = raw.splitn(8, |&b| b == b',').collect();
        ensure!(
            fields.len() == 7 && fields[..3] == [b"57".as_slice(), b"-1", b"4"],
            "unsupported Midori control part definition"
        );
        let slot = u8::try_from(number(fields[3])?)?;
        ensure!(slot < 32, "unsupported Midori control part slot");
        let position = [number(fields[4])?, number(fields[5])?];
        Ok(Self {
            slot,
            position,
            asset_id: Self::asset(fields[6])?,
        })
    }

    pub fn path(&self) -> Result<VfsPath> {
        Ok(VfsPath::new(
            Mount::Asset,
            &format!("res/g/sy/01/01_{}.gyu", self.asset_id),
        )?)
    }

    #[cfg(test)]
    pub fn stock_input(
        &self,
        storage: &dyn crysirror_core::service::ClientStorage,
    ) -> Result<crysirror_core::image_processing::ImageInput> {
        let path = self.path()?;
        let reader = storage.open_read(&path)?;
        let bytes = crysirror_resource::read_all(reader.as_ref(), 1 << 20)?;
        Ok(crysirror_core::image_processing::ImageInput::Encoded {
            name: path.relative().to_owned(),
            bytes: bytes.into(),
            size: [286, 38],
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Parts(pub Vec<Part>);
impl Parts {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        let fields: Vec<_> = raw.splitn(26, |&b| b == b',').collect();
        ensure!(
            fields.len() == 25 && fields[0] == b"3",
            "unsupported Midori bundled control parts"
        );
        let mut parts = Vec::new();
        for (slot, tuple) in fields[1..].chunks_exact(3).enumerate() {
            let [path, x, y] = tuple else { unreachable!() };
            if *path != b"-1" {
                parts.push(Part {
                    slot: slot as u8,
                    position: [number(x)?, number(y)?],
                    asset_id: Part::asset(path)?,
                });
            }
        }
        Ok(Self(parts))
    }
}

/// One plain list update. The resident uses a separate slash/condition-chain
/// mode; that mode and its possible per-slot auxiliary flags are not admitted.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct SoundUpdate {
    pub execute: bool,
    pub first: u8,
    pub values: Vec<i32>,
}
impl SoundUpdate {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        let fields: Vec<_> = raw.splitn(8, |&b| b == b',').collect();
        ensure!(
            fields.len() == 8
                && fields[..3] == [b"338".as_slice(), b"-1", b"0"]
                && fields[4..6] == [b"0".as_slice(), b"0"],
            "unsupported Midori control sound list form"
        );
        let first = u8::try_from(number(fields[3])?)?;
        ensure!(
            usize::from(first) < SOUND_SLOTS,
            "unsupported Midori control sound slot"
        );
        let execute = match fields[6] {
            b"true" => true,
            b"false" => false,
            _ => anyhow::bail!("unsupported Midori control sound list condition"),
        };
        let values = fields[7]
            .split(|&b| b == b'.')
            .map(number)
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            !values.is_empty()
                && usize::from(first) + values.len() <= SOUND_SLOTS
                && values.iter().all(|&id| (-1..=999).contains(&id)),
            "unsupported Midori control sound list extent or ID"
        );
        Ok(Self {
            execute,
            first,
            values,
        })
    }

    #[cfg(test)]
    pub fn stock_audio_paths(&self) -> Result<Vec<VfsPath>> {
        self.values
            .iter()
            .copied()
            .filter(|&id| id >= 0)
            .map(|id| {
                VfsPath::new(Mount::Asset, &format!("res/s/e/{id:03}.wav")).map_err(Into::into)
            })
            .collect()
    }
}

#[derive(Default)]
pub(super) struct Definitions {
    parts: BTreeMap<u8, Part>,
    notifications: BTreeMap<u8, Notification>,
    controls: BTreeMap<u8, crate::v3_control::Definition>,
    // Missing/unconfigured and explicit silent (-1) are distinct. No
    // fabricated native constructor/default table enters this owner.
    pub sounds: crate::midori_control_sound::Catalog,
}
impl Definitions {
    pub fn set_notification(&mut self, notification: Notification) {
        self.notifications.insert(notification.slot, notification);
    }

    pub fn set_control(&mut self, definition: crate::v3_control::Definition) {
        self.controls.insert(definition.slot, definition);
    }

    pub fn control_slots(&self) -> &BTreeMap<u8, crate::v3_control::Definition> {
        &self.controls
    }

    pub fn notification_slots(&self) -> &BTreeMap<u8, Notification> {
        &self.notifications
    }

    pub fn set_part(&mut self, part: Part) {
        self.parts.insert(part.slot, part);
    }
    pub fn set_parts(&mut self, parts: Parts) {
        for part in parts.0 {
            self.set_part(part);
        }
    }
    pub fn update_sound(&mut self, update: SoundUpdate) {
        if update.execute {
            for (slot, id) in (update.first..).zip(update.values) {
                self.sounds.plain(slot, id);
            }
        }
    }
    pub fn sound_slots(&self) -> &BTreeMap<u8, i32> {
        &self.sounds.ids
    }
    pub fn part_status(&self) -> Result<Vec<(u8, String, [i32; 2])>> {
        self.parts
            .values()
            .map(|part| Ok((part.slot, part.path()?.relative().to_owned(), part.position)))
            .collect()
    }
}
