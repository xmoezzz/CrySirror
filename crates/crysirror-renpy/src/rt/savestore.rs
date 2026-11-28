// SPDX-License-Identifier: MPL-2.0
//! Save storage of the native runtime: the client's Save mount (through
//! `ClientStorage`, atomic writes) or, without a host, an in-memory store;
//! and the zip container Ren'Py save files use (`zipfile` with
//! `ZIP_DEFLATED` members; stored members are read too).
use crysirror_core::{
    path::{Mount, VfsPath},
    service::ClientStorage,
};
use flate2::{Compression, read::DeflateDecoder, write::DeflateEncoder};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    io::{Read, Write},
    rc::Rc,
    sync::Arc,
};

/// Largest save-storage file the runtime reads.
pub const MAX_FILE: u64 = 256 * 1024 * 1024;

pub enum SaveStore {
    Client(Arc<dyn ClientStorage>),
    /// name -> (bytes, mtime seconds).
    Memory(RefCell<BTreeMap<String, (Vec<u8>, f64)>>, RefCell<f64>),
}

impl SaveStore {
    pub fn memory() -> SaveStore {
        SaveStore::Memory(RefCell::default(), RefCell::new(1_000_000.0))
    }

    fn path(name: &str) -> Result<VfsPath, String> {
        VfsPath::new(Mount::Save, name).map_err(|e| e.to_string())
    }

    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        match self {
            SaveStore::Client(s) => {
                if let Ok(Some(m)) = s.metadata(Mount::Save, name) {
                    if m.is_directory || m.size.is_some_and(|z| z > MAX_FILE) {
                        return None;
                    }
                }
                s.read(&Self::path(name).ok()?).ok()
            }
            SaveStore::Memory(m, _) => m.borrow().get(name).map(|(d, _)| d.clone()),
        }
    }

    pub fn write(&self, name: &str, data: &[u8]) -> Result<(), String> {
        match self {
            SaveStore::Client(s) => {
                if let Some((dir, _)) = name.rsplit_once('/') {
                    let _ = s.create_directory(&Self::path(dir)?);
                }
                s.write_atomic(&Self::path(name)?, data)
                    .map_err(|e| e.to_string())
            }
            SaveStore::Memory(m, clock) => {
                let mut c = clock.borrow_mut();
                *c += 1.0;
                m.borrow_mut().insert(name.to_owned(), (data.to_vec(), *c));
                Ok(())
            }
        }
    }

    pub fn remove(&self, name: &str) -> Result<(), String> {
        match self {
            SaveStore::Client(s) => match s.remove(&Self::path(name)?) {
                Ok(()) | Err(crysirror_core::Error::NotFound) => Ok(()),
                Err(e) => Err(e.to_string()),
            },
            SaveStore::Memory(m, _) => {
                m.borrow_mut().remove(name);
                Ok(())
            }
        }
    }

    /// File names directly inside `dir` ("" is the root).
    pub fn list(&self, dir: &str) -> Vec<String> {
        match self {
            SaveStore::Client(s) => s.list(Mount::Save, dir).unwrap_or_default(),
            SaveStore::Memory(m, _) => {
                let prefix = if dir.is_empty() {
                    String::new()
                } else {
                    format!("{}/", dir.trim_end_matches('/'))
                };
                m.borrow()
                    .keys()
                    .filter_map(|k| k.strip_prefix(&prefix))
                    .filter(|k| !k.contains('/'))
                    .map(str::to_owned)
                    .collect()
            }
        }
    }

    /// Modification time in seconds.
    pub fn mtime(&self, name: &str) -> Option<f64> {
        match self {
            SaveStore::Client(s) => {
                let m = s.metadata(Mount::Save, name).ok()??;
                if m.is_directory {
                    return None;
                }
                Some(m.modified_ms.map(|ms| ms as f64 / 1000.0).unwrap_or(0.0))
            }
            SaveStore::Memory(m, _) => m.borrow().get(name).map(|(_, t)| *t),
        }
    }
}

/// The runtime's save store (`rt::Host::saves`, else in memory).
pub fn store(vm: &crysirror_python::Vm) -> Rc<SaveStore> {
    let host = super::host(vm);
    let mut h = host.borrow_mut();
    if let Some(s) = &h.save_store {
        return s.clone();
    }
    let s = Rc::new(match &h.saves {
        Some(s) => SaveStore::Client(s.clone()),
        None => SaveStore::memory(),
    });
    h.save_store = Some(s.clone());
    s
}

// ---------------------------------------------------------------------------
// Zip container.

pub struct ZipEntry {
    pub name: String,
    pub data: Vec<u8>,
}

fn u16le(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32le(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// Reads every member of a zip archive (central directory order).
pub fn zip_read(b: &[u8]) -> Result<Vec<ZipEntry>, String> {
    let bad = || "not a zip file".to_string();
    let min = b.len().saturating_sub(22 + 65535);
    let eocd = (min..=b.len().saturating_sub(22))
        .rev()
        .find(|&i| u32le(b, i) == Some(0x0605_4b50))
        .ok_or_else(bad)?;
    let count = u16le(b, eocd + 10).ok_or_else(bad)? as usize;
    let mut off = u32le(b, eocd + 16).ok_or_else(bad)? as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if u32le(b, off) != Some(0x0201_4b50) {
            return Err(bad());
        }
        let method = u16le(b, off + 10).ok_or_else(bad)?;
        let csize = u32le(b, off + 20).ok_or_else(bad)? as usize;
        let usize_ = u32le(b, off + 24).ok_or_else(bad)? as u64;
        let nlen = u16le(b, off + 28).ok_or_else(bad)? as usize;
        let elen = u16le(b, off + 30).ok_or_else(bad)? as usize;
        let clen = u16le(b, off + 32).ok_or_else(bad)? as usize;
        let local = u32le(b, off + 42).ok_or_else(bad)? as usize;
        let name =
            String::from_utf8_lossy(b.get(off + 46..off + 46 + nlen).ok_or_else(bad)?).into_owned();
        off += 46 + nlen + elen + clen;
        if u32le(b, local) != Some(0x0403_4b50) {
            return Err(bad());
        }
        let lnlen = u16le(b, local + 26).ok_or_else(bad)? as usize;
        let lelen = u16le(b, local + 28).ok_or_else(bad)? as usize;
        let start = local + 30 + lnlen + lelen;
        let raw = b.get(start..start + csize).ok_or_else(bad)?;
        if usize_ > MAX_FILE {
            return Err(format!("zip member {name} is too large"));
        }
        let data = match method {
            0 => raw.to_vec(),
            8 => {
                let mut d = Vec::with_capacity(usize_ as usize);
                DeflateDecoder::new(raw)
                    .take(MAX_FILE)
                    .read_to_end(&mut d)
                    .map_err(|e| e.to_string())?;
                d
            }
            m => return Err(format!("zip member {name} uses unsupported method {m}")),
        };
        out.push(ZipEntry { name, data });
    }
    Ok(out)
}

pub fn zip_member(b: &[u8], name: &str) -> Option<Vec<u8>> {
    zip_read(b)
        .ok()?
        .into_iter()
        .find(|e| e.name == name)
        .map(|e| e.data)
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &x in data {
        c ^= x as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0xedb8_8320
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// Writes a zip archive with deflated members (`ZIP_DEFLATED`).
pub fn zip_write(entries: &[(&str, &[u8])], dos_time: (u16, u16)) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let mut e = DeflateEncoder::new(Vec::new(), Compression::new(6));
        let _ = e.write_all(data);
        let comp = e.finish().unwrap_or_default();
        let crc = crc32(data);
        let local = out.len() as u32;
        let header = |sig: u32, central: bool| -> Vec<u8> {
            let mut h = Vec::new();
            h.extend_from_slice(&sig.to_le_bytes());
            if central {
                h.extend_from_slice(&20u16.to_le_bytes());
            }
            h.extend_from_slice(&20u16.to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            h.extend_from_slice(&8u16.to_le_bytes());
            h.extend_from_slice(&dos_time.1.to_le_bytes());
            h.extend_from_slice(&dos_time.0.to_le_bytes());
            h.extend_from_slice(&crc.to_le_bytes());
            h.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            if central {
                h.extend_from_slice(&[0; 6]);
                h.extend_from_slice(&0u32.to_le_bytes());
                h.extend_from_slice(&local.to_le_bytes());
            }
            h.extend_from_slice(name.as_bytes());
            h
        };
        out.extend(header(0x0403_4b50, false));
        out.extend(&comp);
        central.extend(header(0x0201_4b50, true));
    }
    let cd = out.len() as u32;
    out.extend(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// MS-DOS (date, time) of a Unix timestamp (UTC).
pub fn dos_time(unix: f64) -> (u16, u16) {
    let secs = unix.max(315_532_800.0) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, rem % 3600 / 60, rem % 60);
    // Civil date from days since 1970-01-01.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    if mo <= 2 {
        y += 1;
    }
    let date = (((y - 1980).clamp(0, 127) as u16) << 9) | ((mo as u16) << 5) | d as u16;
    let time = ((h as u16) << 11) | ((m as u16) << 5) | (s as u16 / 2);
    (date, time)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_round_trips() {
        let z = zip_write(
            &[("log", b"hello hello hello"), ("json", b"{}")],
            dos_time(1_700_000_000.0),
        );
        let e = zip_read(&z).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].name, "log");
        assert_eq!(e[0].data, b"hello hello hello");
        assert_eq!(zip_member(&z, "json").unwrap(), b"{}");
    }
}
