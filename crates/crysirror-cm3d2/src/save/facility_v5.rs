// SPDX-License-Identifier: MPL-2.0
//! COM3D2 1.0.4 FacilityManager.Serialize (v1040, facility layout "5").
//! A section starts at the reader position supplied by GameMain after ScenarioSelectMgr.
//! Rooms use the native sparse index; pending removal is distinct from no pending change.
use crate::{
    binio::{Reader, Writer},
    params::Exp,
};
use anyhow::{Context, Result, ensure};
use std::collections::HashSet;

const MANAGER: &str = "CM3D21_FACILITY_MGR";
const FLAGS: &str = "CM3D21_TEMP_FLAG";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Room {
    pub guid: String,
    pub type_id: i32,
    pub name: String,
    pub parameters: [i32; 3],
    pub recipes: Vec<(i32, Vec<i32>)>,
    pub costume_type: i32,
    pub costume_parts: Vec<(String, bool, String)>,
    pub day: Vec<String>,
    pub night: Vec<String>,
    pub previous_day: Vec<String>,
    pub previous_night: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Facilities {
    pub rooms: Vec<(i32, Room)>,
    pub materials: Vec<(i32, bool)>,
    pub pending: Vec<(i32, Option<Room>)>,
    pub experience: Vec<(i32, Exp)>,
    pub build_counts: Vec<(i32, i32)>,
    pub achievements: Vec<(String, String)>,
}

fn read_vec<T>(r: &mut Reader, mut read: impl FnMut(&mut Reader) -> Result<T>) -> Result<Vec<T>> {
    let len = r.count()?;
    let mut values = Vec::new();
    for _ in 0..len {
        values.push(read(r)?);
    }
    Ok(values)
}

fn write_vec<T>(
    w: &mut Writer,
    v: &[T],
    mut write: impl FnMut(&mut Writer, &T) -> Result<()>,
) -> Result<()> {
    w.i32(v.len().try_into()?);
    for value in v {
        write(w, value)?;
    }
    Ok(())
}

impl Room {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let guid = r.str()?;
        let type_id = r.i32()?;
        let name = r.str()?;
        let parameters = [r.i32()?, r.i32()?, r.i32()?];
        let recipes = read_vec(r, |r| Ok((r.i32()?, read_vec(r, |r| r.i32())?)))?;
        let costume_type = r.i32()?;
        ensure!(
            (0..=2).contains(&costume_type),
            "unknown facility costume type {costume_type}"
        );
        let costume_parts = read_vec(r, |r| Ok((r.str()?, r.bool()?, r.str()?)))?;
        let mut list = || read_vec(r, |r| r.str());
        Ok(Self {
            guid,
            type_id,
            name,
            parameters,
            recipes,
            costume_type,
            costume_parts,
            day: list()?,
            night: list()?,
            previous_day: list()?,
            previous_night: list()?,
        })
    }

    pub fn write(&self, w: &mut Writer) -> Result<()> {
        ensure!(
            (0..=2).contains(&self.costume_type),
            "unknown facility costume type"
        );
        w.str(&self.guid);
        w.i32(self.type_id);
        w.str(&self.name);
        for p in self.parameters {
            w.i32(p);
        }
        write_vec(w, &self.recipes, |w, (id, materials)| {
            w.i32(*id);
            write_vec(w, materials, |w, id| {
                w.i32(*id);
                Ok(())
            })
        })?;
        w.i32(self.costume_type);
        write_vec(w, &self.costume_parts, |w, (mpn, on, filename)| {
            w.str(mpn);
            w.bool(*on);
            w.str(filename);
            Ok(())
        })?;
        for group in [
            &self.day,
            &self.night,
            &self.previous_day,
            &self.previous_night,
        ] {
            write_vec(w, group, |w, guid| {
                w.str(guid);
                Ok(())
            })?;
        }
        Ok(())
    }
}

pub(super) fn read_flags<T>(r: &mut Reader, read: impl FnMut(&mut Reader) -> Result<T>) -> Result<Vec<T>> {
    r.expect_str(FLAGS)?;
    read_vec(r, read)
}
pub(super) fn write_flags<T>(
    w: &mut Writer,
    records: &[T],
    write: impl FnMut(&mut Writer, &T) -> Result<()>,
) -> Result<()> {
    w.str(FLAGS);
    write_vec(w, records, write)
}

fn room_indices<T>(rooms: &[(i32, T)]) -> Result<()> {
    let mut numbers = HashSet::new();
    ensure!(
        rooms
            .iter()
            .all(|(n, _)| (0..12).contains(n) && numbers.insert(*n)),
        "invalid or duplicate facility slot"
    );
    Ok(())
}

impl Facilities {
    pub fn read(input: &mut Reader) -> Result<Self> {
        // Publish the cursor only after the entire section (including the final
        // flag dictionary) has parsed. The next native section follows directly.
        let start = input.at;
        let parsed = (|| {
            input.expect_str(MANAGER)?;
            ensure!(input.i32()? == 1040, "unsupported facility engine version");
            input.expect_str("5")?;
            let rooms = read_vec(input, |r| Ok((r.i32()?, Room::read(r)?)))?;
            room_indices(&rooms)?;
            let materials = read_flags(input, |r| Ok((r.i32()?, r.bool()?)))?;
            let pending = read_vec(input, |r| {
                Ok((
                    r.i32()?,
                    if r.bool()? {
                        Some(Room::read(r)?)
                    } else {
                        None
                    },
                ))
            })?;
            room_indices(&pending)?;
            let experience = read_flags(input, |r| {
                let key = r.i32()?;
                let mut exp = Exp::default();
                exp.read(r)?;
                Ok((key, exp))
            })?;
            let build_counts = read_flags(input, |r| Ok((r.i32()?, r.i32()?)))?;
            let achievements = read_flags(input, |r| Ok((r.str()?, r.str()?)))?;
            Ok(Self {
                rooms,
                materials,
                pending,
                experience,
                build_counts,
                achievements,
            })
        })();
        if parsed.is_err() {
            input.at = start;
        }
        parsed.context("COM3D2 1.0.4 facility section")
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        room_indices(&self.rooms)?;
        room_indices(&self.pending)?;
        let mut w = Writer::default();
        w.str(MANAGER);
        w.i32(1040);
        w.str("5");
        write_vec(&mut w, &self.rooms, |w, (slot, room)| {
            w.i32(*slot);
            room.write(w)
        })?;
        write_flags(&mut w, &self.materials, |w, (id, enabled)| {
            w.i32(*id);
            w.bool(*enabled);
            Ok(())
        })?;
        write_vec(&mut w, &self.pending, |w, (slot, next)| {
            w.i32(*slot);
            w.bool(next.is_some());
            if let Some(room) = next {
                room.write(w)?;
            }
            Ok(())
        })?;
        write_flags(&mut w, &self.experience, |w, (id, exp)| {
            w.i32(*id);
            exp.write(w);
            Ok(())
        })?;
        write_flags(&mut w, &self.build_counts, |w, (id, count)| {
            w.i32(*id);
            w.i32(*count);
            Ok(())
        })?;
        write_flags(&mut w, &self.achievements, |w, (key, value)| {
            w.str(key);
            w.str(value);
            Ok(())
        })?;
        Ok(w.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(s: &str) -> Vec<u8> {
        s.as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    #[test]
    fn installed_managed_writers_match_room_and_manager_bytes() -> Result<()> {
        // Produced by invoking the installed v1040 managed Serialize(BinaryWriter)
        // methods with synthetic fields. No original game or player save data.
        let manager = hex(concat!(
            "13434D334432315F464143494C4954595F4D475210040000013500000000",
            "10434D334432315F54454D505F464C41470200000014000000010A0000000000000000",
            "10434D334432315F54454D505F464C4147010000006400000007000000110000001700000002000000",
            "10434D334432315F54454D505F464C4147010000006400000003000000",
            "10434D334432315F54454D505F464C41470100000012E696BDE8A8ADE5BCB7E58C96E59B9EE695B00135"
        ));
        let room = hex(concat!(
            "0C666978747572652D726F6F6D6400000006E983A8E5B18B01000000020000000300000001000000",
            "0A00000001000000140000000200000000000000010000000E666978747572652D776F726B6572",
            "0000000000000000010000000A6F6C642D776F726B6572"
        ));
        let mut r = Reader::new(&room);
        let room_state = Room::read(&mut r)?;
        assert_eq!(r.at, room.len());
        assert_eq!(room_state.recipes, [(10, vec![20])]);
        assert_eq!(room_state.previous_night, ["old-worker"]);
        let mut bytes = Writer::default();
        room_state.write(&mut bytes)?;
        assert_eq!(bytes.bytes, room);
        let mut r = Reader::new(&manager);
        let mut v = Facilities::read(&mut r)?;
        assert_eq!(r.at, manager.len());
        assert_eq!(v.materials, [(20, true), (10, false)]);
        assert_eq!(v.achievements, [("施設強化回数".into(), "5".into())]);
        assert_eq!(v.experience[0].1.current, 7);
        assert_eq!(v.encode()?, manager);
        v.rooms.push((9, room_state.clone()));
        v.pending.extend([(9, None), (3, Some(room_state))]);
        let mut stream = v.encode()?;
        let end = stream.len();
        stream.extend_from_slice(b"next section");
        let mut r = Reader::new(&stream);
        assert_eq!(Facilities::read(&mut r)?, v);
        assert_eq!(r.at, end);
        for len in 0..end {
            let mut r = Reader::new(&stream[..len]);
            assert!(Facilities::read(&mut r).is_err());
            assert_eq!(r.at, 0);
        }
        v.pending.push((3, None));
        assert!(v.encode().is_err());
        Ok(())
    }
}
