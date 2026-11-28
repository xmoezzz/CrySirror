// SPDX-License-Identifier: MPL-2.0
//! PlayerStatus.Status and its two schedule backup banks in standalone v1040.
use crate::binio::{Reader, Writer};
use crate::params::NightWorkState;
use anyhow::{Context, Result, ensure};

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Slot {
    pub guid: String,
    pub outcome: [i32; 2],
    pub communication: [bool; 2],
    pub backup_status: Vec<(String, i32)>,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Assignment {
    pub guid: String,
    pub work: [i32; 2],
    pub communication: [bool; 2],
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Player {
    pub name: String,
    pub days: i32,
    pub total_purchases: i64,
    pub money: i64,
    pub coins: i64,
    pub club_name: String,
    pub grade: i32,
    pub evaluation: i32,
    pub gauge: i32,
    pub flags: Vec<(String, i32)>,
    pub parts: Vec<(String, bool)>,
    pub items: Vec<(String, bool)>,
    pub shop: Vec<(i32, i32)>,
    pub slots: Vec<Slot>,
    pub backups: Vec<Vec<Assignment>>,
    pub night_jobs: Vec<(i32, NightWorkState)>,
    pub trophies: Vec<i32>,
    pub job_classes: Vec<i32>,
    pub night_classes: Vec<i32>,
}

const MAGIC: &str = "CM3D2_PLAYER_STATUS";
const CHECK: u32 = 3_830_248_615;

fn short_count(r: &mut Reader) -> Result<usize> {
    Ok(i16::from_le_bytes(r.take(2)?.try_into()?).try_into()?)
}

fn records<T>(r: &mut Reader, mut read: impl FnMut(&mut Reader) -> Result<T>) -> Result<Vec<T>> {
    let count = short_count(r)?;
    let mut out = Vec::new();
    for _ in 0..count {
        out.push(read(r)?);
    }
    Ok(out)
}

fn write_records<T>(
    w: &mut Writer,
    records: &[T],
    mut write: impl FnMut(&mut Writer, &T) -> Result<()>,
) -> Result<()> {
    w.bytes
        .extend_from_slice(&i16::try_from(records.len())?.to_le_bytes());
    for record in records {
        write(w, record)?;
    }
    Ok(())
}

impl Player {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let start = r.at;
        let result = Self::read_fields(r);
        if result.is_err() {
            r.at = start;
        }
        result
    }

    fn read_fields(r: &mut Reader) -> Result<Self> {
        r.expect_str(MAGIC)?;
        ensure!(
            r.i32()? == 1040,
            "unsupported standalone player status version"
        );
        let name = r.str()?;
        let days = r.i32()?;
        let total_purchases = r.i64()?;
        let money = r.i64()?;
        let coins = r.i64()?;
        let club_name = r.str()?;
        let grade = r.i32()?;
        let evaluation = r.i32()?;
        let gauge = r.i32()?;
        let flags = records(r, |r| Ok((r.str()?, r.i32()?)))?;
        let parts = records(r, |r| Ok((r.str()?, r.bool()?)))?;
        let items = records(r, |r| Ok((r.str()?, r.bool()?)))?;
        let shop = records(r, |r| Ok((r.i32()?, r.i32()?)))?;
        let slot_count = short_count(r)?;
        ensure!(slot_count <= 40, "too many native schedule slots");
        let mut slots = Vec::new();
        for _ in 0..slot_count {
            let guid = r.str()?;
            let outcome = [r.i32()?, r.i32()?];
            let communication = [r.bool()?, r.bool()?];
            let count = r.count()?;
            let mut backup_status = Vec::new();
            for _ in 0..count {
                backup_status.push((r.str()?, r.i32()?));
            }
            slots.push(Slot {
                guid,
                outcome,
                communication,
                backup_status,
            });
        }
        let backups = records(r, |r| {
            let mut bank = Vec::new();
            for _ in 0..slot_count {
                bank.push(Assignment {
                    guid: r.str()?,
                    work: [r.i32()?, r.i32()?],
                    communication: [r.bool()?, r.bool()?],
                });
            }
            Ok(bank)
        })?;
        let night_jobs = records(r, |r| {
            Ok((
                r.i32()?,
                NightWorkState {
                    work_id: r.i32()?,
                    called_maid_guid: r.str()?,
                    finish: r.bool()?,
                },
            ))
        })?;
        let trophies = records(r, |r| r.i32())?;
        let job_classes = records(r, |r| r.i32())?;
        let night_classes = records(r, |r| r.i32())?;
        ensure!(r.u32()? == CHECK, "invalid native player status check code");
        Ok(Self {
            name,
            days,
            total_purchases,
            money,
            coins,
            club_name,
            grade,
            evaluation,
            gauge,
            flags,
            parts,
            items,
            shop,
            slots,
            backups,
            night_jobs,
            trophies,
            job_classes,
            night_classes,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        ensure!(
            self.slots.len() <= 40 && self.backups.iter().all(|b| b.len() == self.slots.len()),
            "inconsistent native schedule dimensions"
        );
        let mut w = Writer::default();
        w.str(MAGIC);
        w.i32(1040);
        w.str(&self.name);
        w.i32(self.days);
        w.i64(self.total_purchases);
        w.i64(self.money);
        w.i64(self.coins);
        w.str(&self.club_name);
        for n in [self.grade, self.evaluation, self.gauge] {
            w.i32(n);
        }
        write_records(&mut w, &self.flags, |w, (k, v)| {
            w.str(k);
            w.i32(*v);
            Ok(())
        })?;
        for list in [&self.parts, &self.items] {
            write_records(&mut w, list, |w, (k, v)| {
                w.str(k);
                w.bool(*v);
                Ok(())
            })?;
        }
        write_records(&mut w, &self.shop, |w, (id, v)| {
            w.i32(*id);
            w.i32(*v);
            Ok(())
        })?;
        write_records(&mut w, &self.slots, |w, s| {
            w.str(&s.guid);
            for n in s.outcome {
                w.i32(n);
            }
            for b in s.communication {
                w.bool(b);
            }
            w.i32(s.backup_status.len().try_into()?);
            for (k, v) in &s.backup_status {
                w.str(k);
                w.i32(*v);
            }
            Ok(())
        })?;
        write_records(&mut w, &self.backups, |w, bank| {
            for a in bank {
                w.str(&a.guid);
                for n in a.work {
                    w.i32(n);
                }
                for b in a.communication {
                    w.bool(b);
                }
            }
            Ok(())
        })?;
        write_records(&mut w, &self.night_jobs, |w, (key, n)| {
            w.i32(*key);
            w.i32(n.work_id);
            w.str(&n.called_maid_guid);
            w.bool(n.finish);
            Ok(())
        })?;
        for set in [&self.trophies, &self.job_classes, &self.night_classes] {
            write_records(&mut w, set, |w, n| {
                w.i32(*n);
                Ok(())
            })?;
        }
        w.u32(CHECK);
        Ok(w.bytes)
    }
}

pub(super) struct SlotPrefix {
    pub header: super::Header,
    pub has_transferred_characters: bool,
    pub csv_paths: [Vec<String>; 2],
    pub player: Player,
    pub stock_start: usize,
    pub bookmarks: [usize; 2],
}

/// Decode the GameMain and CharacterMgr prefix through player status.
pub(super) fn read_slot_prefix(bytes: &[u8]) -> Result<SlotPrefix> {
    let mut r = Reader::new(bytes);
    r.expect_str("COM3D2_SAVE")?;
    ensure!(r.i32()? == 1040, "unsupported standalone slot version");
    let header = super::read_header(&mut r)?;
    let bookmarks = bookmarks(&mut r, bytes.len())?;
    // Includes use of transferred staff and both installed CSV lists.
    let start = r.at;
    let mut managed = Reader::new(&bytes[start..bookmarks[0]]);
    let has_transferred_characters = managed.bool()?;
    let mut csv_paths = [Vec::new(), Vec::new()];
    for paths in &mut csv_paths {
        let count = managed.count()?;
        *paths = (0..count).map(|_| managed.str()).collect::<Result<_>>()?;
    }
    managed.expect_str("CM3D2_CHR_MGR")?;
    ensure!(
        managed.i32()? == 1040,
        "unsupported standalone CharacterMgr version"
    );
    let player = Player::read(&mut managed)?;
    Ok(SlotPrefix {
        header,
        has_transferred_characters,
        csv_paths,
        player,
        stock_start: start + managed.at,
        bookmarks,
    })
}

/// Read the player and locate CharacterMgr's stock count in a standalone slot.
pub fn read_slot_player(bytes: &[u8]) -> Result<(Player, usize)> {
    let prefix = read_slot_prefix(bytes)?;
    Ok((prefix.player, prefix.stock_start))
}

pub(crate) fn bookmarks(r: &mut Reader, length: usize) -> Result<[usize; 2]> {
    let mut ends = [0; 2];
    for (index, tag) in ["bookmark_kk_01", "bookmark_kk_02"].iter().enumerate() {
        r.expect_str(tag)?;
        let rel = usize::try_from(r.i64()?)?;
        ends[index] =
            r.at.checked_add(rel)
                .filter(|&n| n <= length)
                .context("standalone save bookmark outside file")?;
    }
    ensure!(
        r.at <= ends[0] && ends[0] <= ends[1],
        "inconsistent standalone save bookmarks"
    );
    Ok(ends)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_managed_player_writer_matches_both_schedule_banks() -> Result<()> {
        // PlayerStatus.Status.Serialize(BinaryWriter), invoked on synthetic
        // detached fields with the installed 1.0.4 CLR assembly.
        let reference = concat!(
            "13434d3344325f504c415945525f5354415455531004000009e694afe9858de4baba020000006400",
            "000000000000ea16b04c020000007b0000000000000006e696bde8a8ad02000000120000001f0000",
            "00010009e69982e99693e5b8af020000000100096974656d2e6d656e750001000464656d6f010100",
            "2c0100000200000001000e666978747572652d776f726b6572020000000000000000010100000010",
            "53746172745f6d6f6e65795f746f7030ea0300000200036f6c64b80b00001027000001000e666978",
            "747572652d776f726b6572b90b0000102700000100010010270000102700000e666978747572652d",
            "776f726b65720101001c00000001001c00000001001c000000a7f44ce4",
        );
        let bytes = reference
            .as_bytes()
            .chunks_exact(2)
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
            .collect::<Vec<_>>();
        let mut r = Reader::new(&bytes);
        let p = Player::read(&mut r)?;
        assert_eq!(r.at, bytes.len());
        assert_eq!(p.money, 9_876_543_210);
        assert_eq!((p.days, p.grade, p.gauge), (2, 2, 31));
        assert_eq!(p.slots[0].communication, [false, true]);
        assert_eq!(
            p.slots[0].backup_status,
            [("Start_money_top0".into(), 1002)]
        );
        assert_eq!(p.backups[0][0].work, [3000, 10000]);
        assert_eq!(p.backups[1][0].work, [3001, 10000]);
        assert_eq!(p.night_jobs[0].1.called_maid_guid, "fixture-worker");
        assert!(p.night_jobs[0].1.finish);
        assert_eq!(p.encode()?, bytes);
        for length in 0..bytes.len() {
            let mut r = Reader::new(&bytes[..length]);
            assert!(Player::read(&mut r).is_err());
            assert_eq!(r.at, 0);
        }
        let mut broken = bytes;
        *broken.last_mut().unwrap() ^= 1;
        assert!(Player::read(&mut Reader::new(&broken)).is_err());
        Ok(())
    }
}
