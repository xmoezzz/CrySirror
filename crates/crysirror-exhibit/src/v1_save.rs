// SPDX-License-Identifier: MPL-2.0
//! Original RLD v1 autosave, FCScenario banks and RetouchSaveDataControl.
use crate::registers::{Limits, Registers, Value};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::{
    Error,
    path::{Mount, VfsPath},
    service::{CivilTime, ClientStorage, civil_weekday},
};
use crysirror_resource::read_all;

const MAX_SAVE_BYTES: u64 = 16 << 20;

pub(super) fn system_path(class: &str, mount: Mount) -> Result<VfsPath> {
    ensure!(
        !class.is_empty()
            && class.len() <= 128
            && class
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')),
        "invalid ExHibit save class"
    );
    Ok(VfsPath::new(mount, &format!("{class}.system.dat"))?)
}

fn optional(storage: &dyn ClientStorage, path: &VfsPath) -> Result<Option<Vec<u8>>> {
    match storage.open_read(path) {
        Ok(reader) => Ok(Some(read_all(reader.as_ref(), MAX_SAVE_BYTES)?)),
        Err(Error::NotFound) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(super) fn read_system(
    storage: &dyn ClientStorage,
    class: &str,
    limits: Limits,
) -> Result<Option<Vec<i32>>> {
    read_system_path(storage, &system_path(class, Mount::Save)?, limits)
}

pub(super) fn read_system_path(
    storage: &dyn ClientStorage,
    path: &VfsPath,
    limits: Limits,
) -> Result<Option<Vec<i32>>> {
    let data = match optional(storage, path)? {
        Some(data) => Some(data),
        None => optional(storage, &VfsPath::new(Mount::Asset, path.relative())?)?,
    };
    data.map(|data| parse_integer_bank(&data, limits.sys))
        .transpose()
}

fn append_string(out: &mut Vec<u8>, data: &[u8]) -> Result<()> {
    ensure!(!data.contains(&0), "NUL in ExHibit save string");
    ensure!(
        (out.len() as u64).saturating_add(data.len() as u64 + 5) <= MAX_SAVE_BYTES,
        "oversized ExHibit save data"
    );
    out.extend(u32::try_from(data.len() + 1)?.to_le_bytes());
    out.extend(data);
    out.push(0);
    Ok(())
}

fn word_at(bytes: &mut &[u8]) -> Result<u32> {
    let (head, rest) = bytes
        .split_at_checked(4)
        .context("truncated ExHibit save word")?;
    *bytes = rest;
    Ok(u32::from_le_bytes(head.try_into()?))
}

fn take_string<'a>(bytes: &mut &'a [u8]) -> Result<&'a [u8]> {
    let size = word_at(bytes)? as usize;
    let (field, rest) = bytes
        .split_at_checked(size)
        .context("truncated ExHibit save string")?;
    let value = field
        .strip_suffix(&[0])
        .context("unterminated ExHibit save string")?;
    ensure!(!value.contains(&0), "invalid ExHibit save string");
    *bytes = rest;
    Ok(value)
}

fn parse_integer_bank(mut bytes: &[u8], expected_count: usize) -> Result<Vec<i32>> {
    ensure!(
        take_string(&mut bytes)? == b"FCReg",
        "invalid ExHibit FCReg header"
    );
    let count = word_at(&mut bytes)? as usize;
    ensure!(
        count == expected_count && bytes.len() == count * 4,
        "unexpected ExHibit system register dimensions"
    );
    Ok(bytes
        .chunks_exact(4)
        .map(|bytes| i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .collect())
}

pub(super) fn write_integer_bank(count: usize, mut value: impl FnMut(i32) -> Result<i32>) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(14 + count * 4);
    append_string(&mut out, b"FCReg")?;
    out.extend(u32::try_from(count)?.to_le_bytes());
    for i in 0..count {
        out.extend(value(i as i32)?.to_le_bytes());
    }
    Ok(out)
}

fn dos_key(time: CivilTime) -> Result<u32> {
    // saveNotify passes the UTC FILETIME through SYSTEMTIME and back
    // before packing it, without calling FileTimeToLocalFileTime.
    let time = CivilTime::from_unix_ms(time.client, time.unix_ms, 0)
        .context("invalid ExHibit autosave time")?;
    ensure!(
        (1980..=2107).contains(&time.year)
            && civil_weekday(time.year, time.month, time.day).is_some()
            && time.hour < 24
            && time.minute < 60
            && time.second < 60,
        "host calendar outside ExHibit autosave range"
    );
    Ok((time.year as u32 - 1980) << 25
        | u32::from(time.month) << 21
        | u32::from(time.day) << 16
        | u32::from(time.hour) << 11
        | u32::from(time.minute) << 5
        | u32::from(time.second / 2))
}

pub(super) struct Slot {
    body: Vec<u8>,
    system: Vec<u8>,
    memo: Vec<u8>,
    ring: usize,
    slot: usize,
    pub next: i32,
}

impl Slot {
    pub fn capture(
        registers: &Registers,
        limits: Limits,
        ring: usize,
        memo: Vec<u8>,
    ) -> Result<Self> {
        ensure!(ring != 0, "no configured ExHibit autosave slots");
        let requested = registers.read(Value::System(7))? as usize;
        let slot = if requested < ring { requested } else { 0 };
        let next = ((slot + 1) % ring) as i32;
        let memo = if memo.is_empty() {
            registers.text(1)?.into()
        } else {
            memo
        };

        // The installed scenario constructor supplies empty auxiliary strings.
        // RLD v1 never starts legacy MCI music. Its decoded BGM/SE cue state
        // belongs to the normal registers already captured by blockEntry.
        let mut body = write_integer_bank(limits.regs, |i| registers.read_normal(i))?;
        append_string(&mut body, b"FCStrReg")?;
        body.extend(u32::try_from(limits.strings)?.to_le_bytes());
        for index in 0..limits.strings {
            append_string(&mut body, registers.text(index as i32)?)?;
        }
        append_string(&mut body, b"")?;
        append_string(&mut body, b"")?;
        body.extend([0; 8]);
        append_string(&mut body, b"")?;

        let system = write_integer_bank(limits.sys, |i| match i {
            0 => Ok(0), // flushSysreg writes S0 before serializing FCReg.
            7 => Ok(next),
            _ => registers.read(Value::System(i)),
        })?;
        Ok(Self {
            body,
            system,
            memo,
            ring,
            slot,
            next,
        })
    }

    fn updated_index(&self, old: Option<&[u8]>, key: u32) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut rest = old.unwrap_or_default();
        for slot in 0..self.ring {
            let previous = rest;
            if old.is_some() {
                word_at(&mut rest)?;
                take_string(&mut rest)?;
            }
            if slot == self.slot {
                out.extend(key.to_le_bytes());
                append_string(&mut out, &self.memo)?;
            } else if old.is_some() {
                out.extend(&previous[..previous.len() - rest.len()]);
            } else {
                out.extend(0u32.to_le_bytes());
                append_string(&mut out, b"")?;
            }
        }
        ensure!(rest.is_empty(), "unexpected ExHibit autosave index length");
        Ok(out)
    }

    pub fn write(self, storage: &dyn ClientStorage, class: &str, time: CivilTime) -> Result<()> {
        let _lease = storage.acquire_lease("exhibit-v1-autosave")?;
        let key = dos_key(time)?;
        let body = VfsPath::new(Mount::Save, &format!("user/a{}", self.slot))?;
        let system = system_path(class, Mount::Save)?;
        let index = VfsPath::new(Mount::Save, "user/adb")?;
        let overlay = optional(storage, &index)?;
        let fallback = if overlay.is_none() {
            optional(storage, &VfsPath::new(Mount::Asset, "user/adb")?)?
        } else {
            None
        };
        let updated = self.updated_index(overlay.as_deref().or(fallback.as_deref()), key)?;
        let writes = [
            (&body, optional(storage, &body)?, self.body),
            (&system, optional(storage, &system)?, self.system),
            (&index, overlay, updated),
        ];
        for (i, (path, _, replacement)) in writes.iter().enumerate() {
            if let Err(error) = storage.write_atomic(path, replacement) {
                let mut rollback_error = None;
                for (path, original, _) in writes[..i].iter().rev() {
                    let rollback = match original {
                        Some(data) => storage.write_atomic(path, data),
                        None => storage.remove(path),
                    };
                    if let Err(rollback) = rollback {
                        rollback_error.get_or_insert_with(|| format!("{path:?}: {rollback}"));
                    }
                }
                if let Some(rollback) = rollback_error {
                    bail!("ExHibit autosave commit failed: {error}; rollback of {rollback}");
                }
                return Err(error).context("ExHibit autosave commit failed");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::{
        Result as CoreResult,
        service::{AssetReader, MemoryAssetReader, StorageLease},
        state::ClientId,
    };
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };

    #[derive(Default)]
    struct Files {
        files: Mutex<BTreeMap<(bool, String), Vec<u8>>>,
        fail_index: bool,
    }

    struct Lease;
    impl StorageLease for Lease {}

    impl ClientStorage for Files {
        fn acquire_lease(&self, _: &str) -> CoreResult<Box<dyn StorageLease>> {
            Ok(Box::new(Lease))
        }

        fn read(&self, path: &VfsPath) -> CoreResult<Vec<u8>> {
            self.files
                .lock()
                .unwrap()
                .get(&(path.mount() == Mount::Save, path.relative().into()))
                .cloned()
                .ok_or(Error::NotFound)
        }

        fn open_read(&self, path: &VfsPath) -> CoreResult<Arc<dyn AssetReader>> {
            Ok(Arc::new(MemoryAssetReader(self.read(path)?.into())))
        }

        fn write_atomic(&self, path: &VfsPath, data: &[u8]) -> CoreResult<()> {
            assert_eq!(path.mount(), Mount::Save);
            if self.fail_index && path.relative() == "user/adb" {
                return Err(Error::ReadOnly);
            }
            self.files
                .lock()
                .unwrap()
                .insert((true, path.relative().into()), data.into());
            Ok(())
        }

        fn remove(&self, path: &VfsPath) -> CoreResult<()> {
            assert_eq!(path.mount(), Mount::Save);
            self.files
                .lock()
                .unwrap()
                .remove(&(true, path.relative().into()));
            Ok(())
        }

        fn list(&self, _: Mount, _: &str) -> CoreResult<Vec<String>> {
            Err(Error::Unsupported("test list"))
        }
    }

    fn registers() -> Result<(Registers, Limits)> {
        let limits = Limits {
            regs: 128,
            sys: 128,
            strings: 8,
            cg: 2,
            scene: 2,
            sound: 2,
            characters: 0,
            messages: 32,
            locals: 20,
        };
        Ok((
            Registers::for_scenario(Some(limits), 1, [800, 600])?,
            limits,
        ))
    }

    fn clock() -> CivilTime {
        CivilTime {
            client: ClientId(1),
            unix_ms: 1795797055000,
            utc_offset_seconds: 0,
            year: 2026,
            month: 11,
            day: 27,
            hour: 16,
            minute: 30,
            second: 55,
            millisecond: 0,
        }
    }

    #[test]
    fn original_autosave_writes_banks_memo_and_ring_to_scoped_files() -> Result<()> {
        let files = Files::default();
        let (mut regs, limits) = registers()?;
        regs.scenario_loaded(&VfsPath::new(Mount::Asset, "rld/first.rld")?)?;
        regs.set_system(7, 1)?;
        regs.set_local(90, 12)?;
        regs.mark_message(17)?;
        regs.checkpoint_v1(12, b"My scene", &[0; 19], &[0; 12], [true, false])?;
        Slot::capture(&regs, limits, 2, vec![])?.write(&files, "fixture", clock())?;
        let raw = files.read(&VfsPath::new(Mount::Save, "user/a1")?)?;
        let mut data = raw.as_slice();
        assert_eq!(take_string(&mut data)?, b"FCReg");
        assert_eq!(word_at(&mut data)?, 128);
        assert_eq!(i32::from_le_bytes(data[360..364].try_into()?), 12);
        data = &data[512..];
        assert_eq!(take_string(&mut data)?, b"FCStrReg");
        assert_eq!(word_at(&mut data)?, 8);
        assert!(take_string(&mut data)?.is_empty());
        assert_eq!(take_string(&mut data)?, b"My scene");
        assert_eq!(take_string(&mut data)?, br"rld\first.rld");
        for _ in 0..7 {
            assert!(take_string(&mut data)?.is_empty());
        }
        assert_eq!(data, [0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0]);

        let raw = files.read(&VfsPath::new(Mount::Save, "user/adb")?)?;
        let mut index = raw.as_slice();
        assert_eq!(word_at(&mut index)?, 0);
        assert!(take_string(&mut index)?.is_empty());
        assert_eq!(
            word_at(&mut index)?,
            (46 << 25) | (11 << 21) | (27 << 16) | (16 << 11) | (30 << 5) | 27
        );
        assert_eq!(take_string(&mut index)?, b"My scene");
        assert!(index.is_empty());

        let mut restored = registers()?.0;
        restored.load_system_bank(read_system(&files, "fixture", limits)?.unwrap())?;
        assert_eq!(restored.read(Value::System(7))?, 0);
        assert_eq!(restored.capture_experience()?.cell(10), Some(1 << (17 ^ 7)));
        Ok(())
    }

    #[test]
    fn failed_index_commit_restores_prior_slot_and_system_files() -> Result<()> {
        let files = Files {
            fail_index: true,
            ..Files::default()
        };
        let old = &b"old body"[..];
        let body = VfsPath::new(Mount::Save, "user/a0")?;
        let system = system_path("fixture", Mount::Save)?;
        files.write_atomic(&body, old)?;
        let (regs, limits) = registers()?;
        let error = Slot::capture(&regs, limits, 2, b"memo".to_vec())?
            .write(&files, "fixture", clock())
            .unwrap_err();
        assert!(error.to_string().contains("commit failed"));
        assert_eq!(files.read(&body)?, old);
        assert!(matches!(files.read(&system), Err(Error::NotFound)));
        Ok(())
    }
}
