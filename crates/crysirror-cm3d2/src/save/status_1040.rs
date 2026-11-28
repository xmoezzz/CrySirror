// SPDX-License-Identifier: MPL-2.0
//! The 1.0.4 `MaidStatus.Status.Serialize` record, including the separate
//! legacy class/skill maps and optional `MaidStatus.Old.Status`.
use crate::binio::Writer;
use anyhow::{Context, Result, ensure};
use crysirror_asset::interchange::cm3d2::Reader;
use std::collections::BTreeSet;

const MAGIC: &str = "CM3D2_MAID_STATUS";
const CHECK: u32 = 2_177_629_204;

fn read_i16(r: &mut Reader<'_>) -> Result<i16> {
    Ok(i16::from_le_bytes(r.take(2)?.try_into()?))
}

fn read_some<'a, T>(
    r: &mut Reader<'a>,
    read: impl FnOnce(&mut Reader<'a>) -> Result<T>,
) -> Result<Option<T>> {
    r.bool()?.then(|| read(r)).transpose()
}

fn write_some<T>(
    w: &mut Writer,
    value: &Option<T>,
    write: impl FnOnce(&mut Writer, &T) -> Result<()>,
) -> Result<()> {
    w.bool(value.is_some());
    if let Some(value) = value {
        write(w, value)?;
    }
    Ok(())
}

fn read_list<'a, T>(
    r: &mut Reader<'a>,
    minimum_size: usize,
    mut read: impl FnMut(&mut Reader<'a>) -> Result<T>,
) -> Result<Vec<T>> {
    let count = usize::try_from(read_i16(r)?).context("negative 1.0.4 employee list count")?;
    ensure!(
        count <= r.remaining() / minimum_size,
        "1.0.4 employee list exceeds its section"
    );
    (0..count).map(|_| read(r)).collect()
}

fn write_list<T>(
    w: &mut Writer,
    entries: &[T],
    mut write: impl FnMut(&mut Writer, &T) -> Result<()>,
) -> Result<()> {
    let count = i16::try_from(entries.len()).context("1.0.4 employee list count")?;
    w.bytes.extend_from_slice(&count.to_le_bytes());
    for value in entries {
        write(w, value)?;
    }
    Ok(())
}

fn read_pairs<'a, K: Ord + Clone, V>(
    r: &mut Reader<'a>,
    minimum_size: usize,
    mut key: impl FnMut(&mut Reader<'a>) -> Result<K>,
    mut value: impl FnMut(&mut Reader<'a>) -> Result<V>,
) -> Result<Vec<(K, V)>> {
    let mut keys = BTreeSet::new();
    read_list(r, minimum_size, |r| {
        let key = key(r)?;
        ensure!(keys.insert(key.clone()), "duplicate 1.0.4 employee map key");
        Ok((key, value(r)?))
    })
}

fn write_pairs<K: Ord, V>(
    w: &mut Writer,
    entries: &[(K, V)],
    mut key: impl FnMut(&mut Writer, &K),
    mut value: impl FnMut(&mut Writer, &V) -> Result<()>,
) -> Result<()> {
    let mut keys = BTreeSet::new();
    for (k, _) in entries {
        ensure!(keys.insert(k), "duplicate 1.0.4 employee map key");
    }
    write_list(w, entries, |w, (k, v)| {
        key(w, k);
        value(w, v)
    })
}

// Keep fields in wire order. These flat records contain no length or
// presence fields implied by their Rust layout.
macro_rules! record {
    ($name:ident { $($field:ident : $ty:ty => $read:expr, $write:expr;)* }) => {
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub(crate) struct $name { $(pub $field: $ty,)* }
        impl $name {
            fn read(r: &mut Reader<'_>) -> Result<Self> {
                Ok(Self { $($field: ($read)(r)?,)* })
            }
            fn write(&self, w: &mut Writer) {
                $(($write)(w, &self.$field);)*
            }
        }
    }
}

fn i16(w: &mut Writer, value: &i16) {
    w.bytes.extend_from_slice(&value.to_le_bytes());
}
fn u16(w: &mut Writer, value: &u16) {
    w.bytes.extend_from_slice(&value.to_le_bytes());
}
fn i32(w: &mut Writer, value: &i32) {
    w.i32(*value);
}
fn u32(w: &mut Writer, value: &u32) {
    w.u32(*value);
}
fn i64(w: &mut Writer, value: &i64) {
    w.i64(*value);
}
fn flag(w: &mut Writer, value: &bool) {
    w.bool(*value);
}
fn string(w: &mut Writer, value: &String) {
    w.str(value);
}

record!(Body {
    height: u16 => Reader::u16, u16;
    weight: u16 => Reader::u16, u16;
    bust: u16 => Reader::u16, u16;
    waist: u16 => Reader::u16, u16;
    hip: u16 => Reader::u16, u16;
    cup: String => Reader::string, string;
});

record!(Parameters {
    plays: u16 => Reader::u16, u16;
    night_work_count: u16 => Reader::u16, u16;
    likability: u16 => Reader::u16, u16;
    study_rate: u16 => Reader::u16, u16;
    max_hp: u16 => Reader::u16, u16;
    hp: u16 => Reader::u16, u16;
    max_mind: u16 => Reader::u16, u16;
    mind: u16 => Reader::u16, u16;
    max_reason: u16 => Reader::u16, u16;
    reason: u16 => Reader::u16, u16;
    reception: u16 => Reader::u16, u16;
    care: u16 => Reader::u16, u16;
    lovely: u16 => Reader::u16, u16;
    elegance: u16 => Reader::u16, u16;
    charm: u16 => Reader::u16, u16;
    cooking: u16 => Reader::u16, u16;
    vocal: u16 => Reader::u16, u16;
    dance: u16 => Reader::u16, u16;
    appeal: u16 => Reader::u16, u16;
    desire: u16 => Reader::u16, u16;
    sensitivity: u16 => Reader::u16, u16;
    perversion: u16 => Reader::u16, u16;
    service: u16 => Reader::u16, u16;
    teaching: u16 => Reader::u16, u16;
    people_count: u16 => Reader::u16, u16;
    rank: u16 => Reader::u16, u16;
    sales: i64 => Reader::i64, i64;
    total_sales: i64 => Reader::i64, i64;
    evaluation: u16 => Reader::u16, u16;
    total_evaluation: i64 => Reader::i64, i64;
    leader: bool => Reader::bool, flag;
    first_name_call: bool => Reader::bool, flag;
    status_lock: bool => Reader::bool, flag;
    eye_parts_tab: i16 => read_i16, i16;
    day_job: i32 => Reader::i32, i32;
    night_job: i32 => Reader::i32, i32;
    day_communication: bool => Reader::bool, flag;
    night_communication: bool => Reader::bool, flag;
});

record!(Experience {
    current: i32 => Reader::i32, i32;
    total: i32 => Reader::i32, i32;
    next: i32 => Reader::i32, i32;
    level: i32 => Reader::i32, i32;
});

record!(SkillHead {
    plays: u32 => Reader::u32, u32;
    command_count: u16 => Reader::u16, u16;
    experience_locked: bool => Reader::bool, flag;
});

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Skill {
    pub counters: SkillHead,
    pub experience: Experience,
}

impl Skill {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            counters: SkillHead::read(r)?,
            experience: Experience::read(r)?,
        })
    }
    fn write(&self, w: &mut Writer) {
        self.counters.write(w);
        self.experience.write(w);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClassEntries<T> {
    pub current: Vec<(i32, T)>,
    /// None reuses the same ID in the current map. Some holds a separate
    /// legacy entry with its own experience.
    pub legacy: Vec<(i32, Option<T>)>,
}

impl<T> ClassEntries<T> {
    fn validate(&self) -> Result<()> {
        let ids: BTreeSet<_> = self.current.iter().map(|(id, _)| *id).collect();
        for (id, record) in &self.legacy {
            ensure!(
                ids.contains(id) == record.is_none(),
                "invalid legacy class or skill alias {id}"
            );
        }
        Ok(())
    }

    fn read<'a>(
        r: &mut Reader<'a>,
        size: usize,
        read: fn(&mut Reader<'a>) -> Result<T>,
    ) -> Result<Self> {
        let current = read_pairs(r, 4 + size, Reader::i32, read)?;
        let legacy = read_pairs(r, 5, Reader::i32, |r| read_some(r, read))?;
        let result = Self { current, legacy };
        result.validate()?;
        Ok(result)
    }

    fn write(&self, w: &mut Writer, write: fn(&T, &mut Writer)) -> Result<()> {
        self.validate()?;
        let write = |w: &mut Writer, value: &T| {
            write(value, w);
            Ok(())
        };
        write_pairs(w, &self.current, i32, write)?;
        write_pairs(w, &self.legacy, i32, |w, value| write_some(w, value, write))
    }
}

record!(Work {
    id: i32 => Reader::i32, i32;
    plays: u32 => Reader::u32, u32;
    level: i32 => Reader::i32, i32;
});

record!(OldFields {
    elapsed_days: i32 => Reader::i32, i32;
    condition: i16 => read_i16, i16;
    mouth: i32 => Reader::i32, i32;
    throat: i32 => Reader::i32, i32;
    nipple: i32 => Reader::i32, i32;
    front: i32 => Reader::i32, i32;
    back: i32 => Reader::i32, i32;
    curi: i32 => Reader::i32, i32;
    relation: i16 => read_i16, i16;
    frustration: i16 => read_i16, i16;
    married: bool => Reader::bool, flag;
    newlywed: bool => Reader::bool, flag;
    rental: bool => Reader::bool, flag;
});

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OldStatus {
    pub fields: OldFields,
    pub flags: Vec<(String, i32)>,
}

impl OldStatus {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            fields: OldFields::read(r)?,
            flags: read_pairs(r, 5, Reader::string, Reader::i32)?,
        })
    }
    fn write(&self, w: &mut Writer) -> Result<()> {
        self.fields.write(w);
        write_pairs(w, &self.flags, string, |w, value| {
            w.i32(*value);
            Ok(())
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Heroine {
    #[default]
    Original,
    Sub(i32),
    Transfer,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Status {
    pub guid: String,
    pub creation_time: String,
    pub heroine: Heroine,
    pub main_character: bool,
    pub employment_day: i32,
    pub last_name: String,
    pub first_name: String,
    pub profile: String,
    pub comment: String,
    pub initial_experience: i16,
    pub experience_type: i16,
    pub personality: i32,
    pub contract: i16,
    pub body: Body,
    pub relation: i16,
    pub parameters: Parameters,
    pub features: Vec<i32>,
    pub propensities: Vec<i32>,
    pub job_classes: ClassEntries<Experience>,
    pub night_classes: ClassEntries<Experience>,
    pub skills: ClassEntries<Skill>,
    pub works: Vec<(i32, Work)>,
    pub flags: Vec<(String, i32)>,
    pub events: Vec<(i32, bool)>,
    pub parts: Vec<(String, String)>,
    pub legacy: Option<OldStatus>,
    pub selected_job_class: Option<i32>,
    pub selected_night_class: Option<i32>,
}

impl Status {
    pub(super) fn read(r: &mut Reader<'_>) -> Result<Self> {
        ensure!(
            r.header(MAGIC)? == 1040,
            "unsupported standalone employee status version"
        );
        let guid = r.string()?;
        let creation_time = r.string()?;
        let heroine = match read_i16(r)? {
            0 => Heroine::Original,
            1 => Heroine::Sub(r.i32()?),
            2 => Heroine::Transfer,
            kind => anyhow::bail!("unknown 1.0.4 heroine kind {kind}"),
        };
        let result = Self {
            guid,
            creation_time,
            heroine,
            main_character: r.bool()?,
            employment_day: r.i32()?,
            last_name: r.string()?,
            first_name: r.string()?,
            profile: r.string()?,
            comment: r.string()?,
            initial_experience: read_i16(r)?,
            experience_type: read_i16(r)?,
            personality: r.i32()?,
            contract: read_i16(r)?,
            body: Body::read(r)?,
            relation: read_i16(r)?,
            parameters: Parameters::read(r)?,
            features: read_list(r, 4, Reader::i32)?,
            propensities: read_list(r, 4, Reader::i32)?,
            job_classes: ClassEntries::read(r, 16, Experience::read)?,
            night_classes: ClassEntries::read(r, 16, Experience::read)?,
            skills: ClassEntries::read(r, 23, Skill::read)?,
            works: read_pairs(r, 16, Reader::i32, Work::read)?,
            flags: read_pairs(r, 5, Reader::string, Reader::i32)?,
            events: read_pairs(r, 5, Reader::i32, Reader::bool)?,
            parts: read_pairs(r, 2, Reader::string, Reader::string)?,
            legacy: read_some(r, OldStatus::read)?,
            selected_job_class: read_some(r, Reader::i32)?,
            selected_night_class: read_some(r, Reader::i32)?,
        };
        ensure!(
            r.u32()? == CHECK,
            "invalid standalone employee status end marker"
        );
        Ok(result)
    }

    pub(super) fn write(&self, w: &mut Writer) -> Result<()> {
        w.str(MAGIC);
        w.i32(1040);
        w.str(&self.guid);
        w.str(&self.creation_time);
        match self.heroine {
            Heroine::Original => i16(w, &0),
            Heroine::Sub(id) => {
                i16(w, &1);
                w.i32(id);
            }
            Heroine::Transfer => i16(w, &2),
        }
        w.bool(self.main_character);
        w.i32(self.employment_day);
        for text in [
            &self.last_name,
            &self.first_name,
            &self.profile,
            &self.comment,
        ] {
            w.str(text);
        }
        i16(w, &self.initial_experience);
        i16(w, &self.experience_type);
        w.i32(self.personality);
        i16(w, &self.contract);
        self.body.write(w);
        i16(w, &self.relation);
        self.parameters.write(w);
        let write_id = |w: &mut Writer, id: &i32| {
            w.i32(*id);
            Ok(())
        };
        write_list(w, &self.features, write_id)?;
        write_list(w, &self.propensities, write_id)?;
        self.job_classes.write(w, Experience::write)?;
        self.night_classes.write(w, Experience::write)?;
        self.skills.write(w, Skill::write)?;
        write_pairs(w, &self.works, i32, |w, work| {
            work.write(w);
            Ok(())
        })?;
        write_pairs(w, &self.flags, string, write_id)?;
        write_pairs(w, &self.events, i32, |w, value| {
            w.bool(*value);
            Ok(())
        })?;
        write_pairs(w, &self.parts, string, |w, text| {
            w.str(text);
            Ok(())
        })?;
        write_some(w, &self.legacy, |w, status| status.write(w))?;
        write_some(w, &self.selected_job_class, write_id)?;
        write_some(w, &self.selected_night_class, write_id)?;
        w.u32(CHECK);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn employee_status_matches_installed_serialize() -> Result<()> {
        // Captured by invoking MaidStatus.Status.Serialize(BinaryWriter) in
        // the installed 1.0.4 assembly on synthetic values, without Unity.
        let reference = concat!(
            "11434d3344325f4d4149445f535441545553100400000e666978747572652d77",
            "6f726b65720e3230323631313235303030303030010034000000010700000006",
            "e8a9a6e9a89306e5908de5898d07666978747572650766697874757265000000",
            "00070000000000b1000000000000000000014100000000000061ea0000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000cb04fb711f01000000000000000000000000cc04fb711f",
            "0100000000000000c20b00001027000000010100690000000100cf0000000100",
            "360100000b000000150000001f00000003000000020036010000004001000001",
            "150000001f000000290000000300000001009a0100000b000000150000001f00",
            "00000300000002009a01000000a401000001150000001f000000290000000300",
            "00000100fe0100007b286bee48ee011f00000029000000330000000300000002",
            "00fe0100000008020000010000000000000029000000330000003d0000000300",
            "000001001027000010270000c8296bee0900000001000c666978747572652d66",
            "6c6167f3ffffff0100fd0200000101000c666978747572652d706172740d6669",
            "78747572652d76616c7565011200000000000000000000000000000000000000",
            "0000000000000000000000000e000001000100086f6c642d666c616729000000",
            "0136010000019a01000014fccb81",
        );
        let bytes: Vec<_> = (0..reference.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&reference[i..i + 2], 16))
            .collect::<Result<_, _>>()?;
        let mut r = Reader::new(&bytes);
        let status = Status::read(&mut r)?;
        assert_eq!(r.remaining(), 0);
        assert_eq!(status.heroine, Heroine::Sub(52));
        assert_eq!(status.body.height, 177);
        assert_eq!(status.parameters.likability, 60_001);
        assert_eq!(status.parameters.sales, 1_234_567_890_123);
        assert_eq!(status.parameters.night_job, 10_000);
        assert_eq!(status.features, [105]);
        assert_eq!(status.propensities, [207]);
        assert_eq!(status.job_classes.current[0].1.current, 11);
        assert_eq!(status.job_classes.legacy[0], (310, None));
        assert_eq!(status.job_classes.legacy[1].1.as_ref().unwrap().total, 31);
        assert_eq!(status.night_classes.legacy[1].0, 420);
        assert_eq!(status.skills.current[0].1.counters.plays, 4_000_000_123);
        assert_eq!(status.skills.current[0].1.counters.command_count, 61_000);
        assert!(status.skills.current[0].1.counters.experience_locked);
        assert_eq!(
            status.skills.legacy[1].1.as_ref().unwrap().experience.next,
            61
        );
        assert_eq!(status.works[0].1.plays, 4_000_000_456);
        assert_eq!(status.flags[0], ("fixture-flag".into(), -13));
        assert_eq!(status.events[0], (765, true));
        assert_eq!(
            status.parts[0],
            ("fixture-part".into(), "fixture-value".into())
        );
        let legacy = status.legacy.as_ref().unwrap();
        assert_eq!(legacy.fields.elapsed_days, 18);
        assert_eq!(legacy.flags[0], ("old-flag".into(), 41));
        assert_eq!(
            (status.selected_job_class, status.selected_night_class),
            (Some(310), Some(410))
        );

        let mut w = Writer::default();
        status.write(&mut w)?;
        assert_eq!(w.bytes, bytes);
        *w.bytes.last_mut().unwrap() = 0;
        assert!(Status::read(&mut Reader::new(&w.bytes)).is_err());

        let mut invalid = status;
        invalid.job_classes.legacy[0].0 = 999;
        assert!(invalid.write(&mut Writer::default()).is_err());
        Ok(())
    }
}
