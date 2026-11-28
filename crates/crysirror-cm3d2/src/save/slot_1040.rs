// SPDX-License-Identifier: MPL-2.0
//! Detached GameMain container for the standalone COM3D2 1.0.4 profile.
use super::{
    Header,
    facility_v5::{read_flags, write_flags},
    management_1040::{Management, read_ids, read_versioned, write_ids, write_versioned},
    player_1040,
    stock_1040::{self, Roster},
};
use crate::binio::{Reader, Writer};
use anyhow::{Context, Result, ensure};
use std::collections::BTreeSet;

const KASIZUKI: &str = "COM3D2_KASIZUKI";
const CASINO: &str = "COM3D2_CASINO";
const SHOP_IDS: [&str; 2] = ["COM3D2_CASINOSHOP_ID", "COM3D2_CASINOSHOP_ID_END"];
const GUEST_VERSION: i32 = 5;

/// Guest-panel writes replace only the bookmarked KasizukiManager section.
/// They preserve the campaign checkpoint and casino suffix verbatim.
pub(crate) fn edit_guest_slots(
    bytes: &[u8],
    edit: impl FnOnce(&mut Vec<Guest>) -> Result<()>,
) -> Result<Vec<u8>> {
    let ([start, end], second_length) = guest_bounds(bytes)?;
    let mut guests = Guest::read_section(&bytes[start..end])?;
    edit(&mut guests)?;
    let mut w = Writer::default();
    w.bytes.extend_from_slice(&bytes[..start]);
    Guest::write_section(&mut w, &guests)?;
    patch_bookmark(&mut w, second_length)?;
    w.bytes.extend_from_slice(&bytes[end..]);
    Ok(w.bytes)
}

pub(crate) fn read_guest_slots(bytes: &[u8]) -> Result<Vec<Guest>> {
    let ([start, end], _) = guest_bounds(bytes)?;
    Guest::read_section(&bytes[start..end])
}

fn guest_bounds(bytes: &[u8]) -> Result<([usize; 2], usize)> {
    let mut r = Reader::new(bytes);
    read_versioned(&mut r, "COM3D2_SAVE")?;
    super::read_header(&mut r)?;
    let bounds = player_1040::bookmarks(&mut r, bytes.len())?;
    Ok((bounds, r.at - 8))
}

type Flags = Vec<(String, String)>;

#[derive(Debug, PartialEq)]
pub struct Slot<'a> {
    /// Mutable slot metadata. The encoder relocates both bookmarks if the
    /// encoded header changes length.
    pub header: Header,
    pub(crate) has_transferred_characters: bool,
    pub(crate) csv_paths: [Vec<String>; 2],
    pub(crate) roster: Roster<'a>,
    pub(crate) management: Management<'a>,
    pub(crate) guest_slots: Vec<Guest>,
    pub(crate) casino: Casino,
}

impl<'a> Slot<'a> {
    /// Decode a detached standalone 1.0.4 slot without opening its resources
    /// or executing the embedded KagScript checkpoint.
    pub fn read(bytes: &'a [u8]) -> Result<Self> {
        let prefix = player_1040::read_slot_prefix(bytes)?;
        let [end_management, end_guest] = prefix.bookmarks;
        let roster = stock_1040::read(
            &bytes[prefix.stock_start..end_management],
            prefix.stock_start,
            prefix.player,
        )?;
        let management = Management::read(&bytes[roster.script_offset..end_management])?;
        let guest_slots = Guest::read_section(&bytes[end_management..end_guest])
            .context("1.0.4 KasizukiManager")?;
        let casino = Casino::read(&bytes[end_guest..]).context("1.0.4 CasinoDataMgr")?;
        Ok(Self {
            header: prefix.header,
            has_transferred_characters: prefix.has_transferred_characters,
            csv_paths: prefix.csv_paths,
            roster,
            management,
            guest_slots,
            casino,
        })
    }

    /// Write the native slot body and recalculate both bookmark distances.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = Writer::default();
        write_versioned(&mut w, "COM3D2_SAVE");
        super::write_header(&mut w, &self.header);
        w.str("bookmark_kk_01");
        let management = w.bytes.len();
        w.i64(0);
        w.str("bookmark_kk_02");
        let guest = w.bytes.len();
        w.i64(0);

        w.bool(self.has_transferred_characters);
        for paths in &self.csv_paths {
            w.i32(i32::try_from(paths.len())?);
            for path in paths {
                w.str(path);
            }
        }
        w.bytes.extend(self.roster.encode()?);
        w.bytes.extend(self.management.encode()?);
        patch_bookmark(&mut w, management)?;
        Guest::write_section(&mut w, &self.guest_slots)?;
        patch_bookmark(&mut w, guest)?;
        self.casino.write(&mut w)?;
        Ok(w.bytes)
    }
}

fn patch_bookmark(w: &mut Writer, index: usize) -> Result<()> {
    let length = i64::try_from(w.bytes.len() - (index + 8))?;
    w.bytes[index..index + 8].copy_from_slice(&length.to_le_bytes());
    Ok(())
}

// FacilityFlag.Util.CreatePrefix encodes a nonnegative Int32 in seven-bit groups.
fn read_prefix(r: &mut Reader<'_>) -> Result<usize> {
    let mut value = 0u32;
    for shift in (0..35).step_by(7) {
        let byte = r.take(1)?[0];
        ensure!(shift < 28 || byte <= 7, "1.0.4 guest prefix overflow");
        value |= u32::from(byte & 0x7f) << shift;
        if byte < 128 {
            return Ok(value as usize);
        }
    }
    unreachable!("the final byte must terminate the prefix")
}

fn write_prefix(w: &mut Writer, n: usize) -> Result<()> {
    let mut n = i32::try_from(n)?;
    while n >= 128 {
        w.bytes.push(n as u8 | 0x80);
        n >>= 7;
    }
    w.bytes.push(n as u8);
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Guest {
    pub slot: i32,
    pub save_time: String,
    pub last_man_type: i32,
    pub play_count: i32,
    pub comment: String,
    pub system: Flags,
    pub characters: Vec<(String, Flags)>,
}

impl Guest {
    fn read_section(bytes: &[u8]) -> Result<Vec<Self>> {
        let mut r = Reader::new(bytes);
        read_versioned(&mut r, KASIZUKI)?;
        let count = read_prefix(&mut r)?;
        const MIN_GUEST_HEADER_BYTES: usize = 4 * 4 + 3;
        ensure!(
            count <= (bytes.len() - r.at) / MIN_GUEST_HEADER_BYTES,
            "invalid 1.0.4 guest slot count"
        );
        let mut seen = BTreeSet::new();
        let guests = (0..count)
            .map(|_| {
                let guest = Self::read(&mut r)?;
                ensure!(
                    seen.insert(guest.slot),
                    "duplicate guest slot {}",
                    guest.slot
                );
                Ok(guest)
            })
            .collect::<Result<_>>()?;
        ensure!(
            r.at == bytes.len(),
            "guest section does not end at bookmark 2"
        );
        Ok(guests)
    }

    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let slot = r.i32()?;
        ensure!(r.i32()? == GUEST_VERSION, "unsupported guest slot version");
        let save_time = r.str()?;
        let last_man_type = r.i32()?;
        let play_count = r.i32()?;
        let comment = r.str()?;
        let size = read_prefix(r)?;
        let data = r.take(size)?;
        let mut r = Reader::new(data);
        let system = read_string_flags(&mut r)?;
        let count = r.count()?;
        let mut seen = BTreeSet::new();
        let characters = (0..count)
            .map(|_| {
                let guid = r.str()?;
                ensure!(
                    seen.insert(guid.clone()),
                    "duplicate guest character {guid}"
                );
                Ok((guid, read_string_flags(&mut r)?))
            })
            .collect::<Result<_>>()?;
        ensure!(r.at == size, "unread bytes in guest slot payload");
        Ok(Self {
            slot,
            save_time,
            last_man_type,
            play_count,
            comment,
            system,
            characters,
        })
    }

    fn write_section(w: &mut Writer, guests: &[Self]) -> Result<()> {
        write_versioned(w, KASIZUKI);
        write_prefix(w, guests.len())?;
        for guest in guests {
            guest.write(w)?;
        }
        Ok(())
    }

    fn write(&self, w: &mut Writer) -> Result<()> {
        w.i32(self.slot);
        w.i32(GUEST_VERSION);
        w.str(&self.save_time);
        w.i32(self.last_man_type);
        w.i32(self.play_count);
        w.str(&self.comment);
        let mut payload = Writer::default();
        write_string_flags(&mut payload, &self.system)?;
        payload.i32(i32::try_from(self.characters.len())?);
        for (guid, flags) in &self.characters {
            payload.str(guid);
            write_string_flags(&mut payload, flags)?;
        }
        write_prefix(w, payload.bytes.len())?;
        w.bytes.extend(payload.bytes);
        Ok(())
    }
}

fn read_string_flags(r: &mut Reader<'_>) -> Result<Flags> {
    let mut seen = BTreeSet::new();
    read_flags(r, |r| {
        let key = r.str()?;
        ensure!(seen.insert(key.clone()), "duplicate guest flag {key}");
        Ok((key, r.str()?))
    })
}

fn write_string_flags(w: &mut Writer, flags: &Flags) -> Result<()> {
    write_flags(w, flags, |w, (key, value)| {
        w.str(key);
        w.str(value);
        Ok(())
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Dealer {
    pub active_index: i32,
    pub stock_index: i32,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Casino {
    version: String,
    pub low: Option<Dealer>,
    pub high: Option<Dealer>,
    pub sold_out: Vec<(i32, bool)>,
}

impl Casino {
    pub(crate) fn from_parts(
        low: Option<Dealer>,
        high: Option<Dealer>,
        sold_out: Vec<(i32, bool)>,
    ) -> Self {
        Self {
            version: "Ver:1.1".into(),
            low,
            high,
            sold_out,
        }
    }

    fn read(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        r.expect_str(CASINO)?;
        let version = r.str()?;
        ensure!(
            matches!(version.as_str(), "Ver:1.1" | "Ver:1,1"),
            "unsupported 1.0.4 casino state version"
        );
        r.expect_str("Low")?;
        let low = Self::read_dealer(&mut r, bytes, "High")?;
        r.expect_str("High")?;
        let high = Self::read_dealer(&mut r, bytes, SHOP_IDS[0])?;
        let mut seen = BTreeSet::new();
        let sold_out = read_ids(&mut r, SHOP_IDS)?
            .into_iter()
            .map(|id| {
                ensure!(seen.insert(id), "duplicate casino shop item {id}");
                Ok((id, r.bool()?))
            })
            .collect::<Result<_>>()?;
        ensure!(r.at == bytes.len(), "unexpected casino section suffix");
        Ok(Self {
            version,
            low,
            high,
            sold_out,
        })
    }

    fn read_dealer(r: &mut Reader<'_>, bytes: &[u8], next: &str) -> Result<Option<Dealer>> {
        // Native presence depends on whether the corresponding facility exists.
        // The next tag's length and first letters cannot encode a valid active
        // slot number, so the following native section disambiguates it on disk.
        let mut probe = Reader::new(&bytes[r.at..]);
        if probe.expect_str(next).is_ok() {
            return Ok(None);
        }
        let active_index = r.i32()?;
        let stock_index = r.i32()?;
        ensure!(
            active_index >= -1 && stock_index >= -1 && (active_index == -1 || stock_index == -1),
            "invalid casino dealer character indices"
        );
        Ok(Some(Dealer {
            active_index,
            stock_index,
        }))
    }

    fn write(&self, w: &mut Writer) -> Result<()> {
        w.str(CASINO);
        w.str(&self.version);
        for (tag, dealer) in [("Low", &self.low), ("High", &self.high)] {
            w.str(tag);
            if let Some(dealer) = dealer {
                w.i32(dealer.active_index);
                w.i32(dealer.stock_index);
            }
        }
        write_ids(w, SHOP_IDS, self.sold_out.iter().map(|(id, _)| *id));
        for (_, sold_out) in &self.sold_out {
            w.bool(*sold_out);
        }
        Ok(())
    }
}
