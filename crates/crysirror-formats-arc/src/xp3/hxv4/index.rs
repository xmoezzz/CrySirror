// Portions adapted from xmoezzz/xp3-brute, revision c82cb14302138125640478556723705aa720007b.
// Copyright (c) xp3-brute contributors; modifications CrySirror contributors.
// MIT License: see NOTICE-MIT in this directory.
#![allow(dead_code)]
use anyhow::Result;
use chacha20poly1305::{aead::Aead, KeyInit, XChaCha20Poly1305, XNonce};
use flate2::read::ZlibDecoder;
use std::{collections::BTreeMap, io::Read};
const HXV4_SALT: &str = "xp3hnp";
#[derive(Clone, Debug)]
pub struct Hxv4IndexKeys {
    pub key: [u8; 32],
    pub nonce: [u8; 24],
}
#[derive(Clone, Debug)]
pub struct Hxv4Index {
    pub entries: Vec<Hxv4IndexEntry>,
    pub decompressed_size: usize,
}
pub fn hxv4_special_nonce_slot(flags: u16) -> usize {
    (flags & 1) as usize
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hxv4IndexEntry {
    /// Zero-based record number in the decoded Special table.
    pub record_index: usize,
    /// Raw first 64-bit integer from the authenticated Special record.
    ///
    /// Native `sub_10005850` consumes this integer as two independent dwords:
    /// the low dword is the synthetic XP3 lookup id, while the high dword is a
    /// packed locator/control value.
    pub packed: u64,
    /// High 16 bits of `packed >> 32`; selects the archive slot.
    pub archive_slot: u16,
    /// Low 16 bits of `packed >> 32`. `sub_10005850` writes this value to the
    /// fourth output parameter and `sub_10013CF0` uses bit 0 as `open_flag`.
    /// The historical field name is retained for API compatibility.
    pub filter_flag: u16,
    /// Low 32 bits of `packed`. This is passed to `sub_100179A0` and encoded as
    /// the synthetic Unicode XP3 `info` name.
    pub id: u64,
    pub entry_key: u64,
    pub path_hash: [u8; 8],
    pub name_hash: [u8; 32],
    pub path: Option<String>,
    pub name: Option<String>,
}

pub fn decrypt_hxv4_special_plaintext(blob: &[u8], keys: &Hxv4IndexKeys) -> Result<Vec<u8>> {
    if blob.len() < 21 {
        return Err(anyhow::anyhow!("Hxv4 special index is too short"));
    }
    let cipher = XChaCha20Poly1305::new_from_slice(&keys.key)
        .map_err(|_| anyhow::anyhow!("invalid Hxv4 XChaCha20 key length"))?;
    let mut sealed = Vec::with_capacity(blob.len());
    sealed.extend_from_slice(&blob[16..]);
    sealed.extend_from_slice(&blob[..16]);
    cipher
        .decrypt(XNonce::from_slice(&keys.nonce), sealed.as_ref())
        .map_err(|_| anyhow::anyhow!("Hxv4 XChaCha20-Poly1305 authentication failed"))
}

/// Authenticate, decrypt, and zlib-decompress the Hxv4 Special index.
pub fn decrypt_hxv4_special_payload(
    blob: &[u8],
    keys: &Hxv4IndexKeys,
    limit: usize,
) -> Result<Vec<u8>> {
    let decrypted = decrypt_hxv4_special_plaintext(blob, keys)?;
    if decrypted.len() <= 4 {
        return Err(anyhow::anyhow!("Hxv4 special-index plaintext is truncated"));
    }

    let expected = u32::from_le_bytes(decrypted[..4].try_into().unwrap()) as usize;
    if expected == 0 || expected > limit.min(16 * 1024 * 1024) {
        return Err(anyhow::anyhow!(format!(
            "Hxv4 special-index uncompressed size is unreasonable: {expected}"
        )));
    }
    let mut decoder = ZlibDecoder::new(&decrypted[4..]).take(expected as u64 + 1);
    let mut inflated = Vec::with_capacity(expected.min(16 * 1024 * 1024));
    decoder
        .read_to_end(&mut inflated)
        .map_err(|e| anyhow::anyhow!(format!("Hxv4 special-index zlib failed: {e}")))?;
    if inflated.len() != expected {
        return Err(anyhow::anyhow!(format!(
            "Hxv4 special-index size mismatch: got {}, expected {}",
            inflated.len(),
            expected
        )));
    }
    Ok(inflated)
}

pub fn decrypt_hxv4_special_index(
    blob: &[u8],
    keys: &Hxv4IndexKeys,
    limit: usize,
) -> Result<Hxv4Index> {
    let inflated = decrypt_hxv4_special_payload(blob, keys, limit)?;
    let mut cursor = Cursor::new(&inflated);
    let root = read_hx_object(&mut cursor, 0)?;
    if cursor.pos != inflated.len() {
        return Err(anyhow::anyhow!(format!(
            "Hxv4 index deserializer left {} trailing bytes",
            inflated.len() - cursor.pos
        )));
    }
    let entries = extract_hx_index_entries(&root)?;
    Ok(Hxv4Index {
        entries,
        decompressed_size: inflated.len(),
    })
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum HxObject {
    Null,
    String(String),
    Bytes(Vec<u8>),
    Int(i64),
    Float(f64),
    Array(Vec<HxObject>),
    Dict(BTreeMap<String, HxObject>),
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    nodes: usize,
}
impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            nodes: 0,
        }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.data.len().saturating_sub(self.pos) {
            return Err(anyhow::anyhow!("truncated Hxv4 index object"));
        }
        let out = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn be_i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn be_i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
}

fn read_hx_object(cursor: &mut Cursor<'_>, depth: usize) -> Result<HxObject> {
    cursor.nodes += 1;
    if cursor.nodes > 262144 {
        return Err(anyhow::anyhow!("Hxv4 object budget"));
    }
    if depth > 64 {
        return Err(anyhow::anyhow!("Hxv4 index nesting limit exceeded"));
    }
    match cursor.u8()? {
        0x00 | 0x01 => Ok(HxObject::Null),
        0x02 => Ok(HxObject::String(read_hx_string(cursor)?)),
        0x03 => {
            let count = checked_count(cursor.be_i32()?, "Hxv4 byte array")?;
            Ok(HxObject::Bytes(cursor.take(count)?.to_vec()))
        }
        0x04 => Ok(HxObject::Int(cursor.be_i64()?)),
        0x05 => Ok(HxObject::Float(f64::from_bits(cursor.be_i64()? as u64))),
        0x81 => {
            let count = checked_count(cursor.be_i32()?, "Hxv4 array")?;
            if count > 262144 {
                return Err(anyhow::anyhow!("Hxv4 array count is unreasonable"));
            }
            if count > cursor.data.len().saturating_sub(cursor.pos) {
                return Err(anyhow::anyhow!("Hxv4 object count exceeds bytes"));
            }
            let mut out = Vec::with_capacity(count);
            for _ in 0..count {
                out.push(read_hx_object(cursor, depth + 1)?);
            }
            Ok(HxObject::Array(out))
        }
        0xC1 => {
            let count = checked_count(cursor.be_i32()?, "Hxv4 dictionary")?;
            if count > 262144 {
                return Err(anyhow::anyhow!("Hxv4 dictionary count is unreasonable"));
            }
            let mut out = BTreeMap::new();
            for _ in 0..count {
                let name = read_hx_string(cursor)?;
                let value = read_hx_object(cursor, depth + 1)?;
                if out.insert(name, value).is_some() {
                    return Err(anyhow::anyhow!("duplicate Hxv4 dictionary key"));
                }
            }
            Ok(HxObject::Dict(out))
        }
        other => Err(anyhow::anyhow!(format!(
            "unknown Hxv4 index object type 0x{other:02x}"
        ))),
    }
}

fn read_hx_string(cursor: &mut Cursor<'_>) -> Result<String> {
    let chars = checked_count(cursor.be_i32()?, "Hxv4 string")?;
    let bytes = chars
        .checked_mul(2)
        .ok_or_else(|| anyhow::anyhow!("Hxv4 string length overflow"))?;
    let raw = cursor.take(bytes)?;
    let words: Vec<u16> = raw
        .chunks_exact(2)
        .map(|p| u16::from_be_bytes([p[0], p[1]]))
        .collect();
    Ok(String::from_utf16(&words).map_err(|e| anyhow::anyhow!(e))?)
}

fn checked_count(value: i32, what: &str) -> Result<usize> {
    if value < 0 {
        return Err(anyhow::anyhow!(format!("negative {what} length")));
    }
    Ok(value as usize)
}

fn extract_hx_index_entries(root: &HxObject) -> Result<Vec<Hxv4IndexEntry>> {
    let HxObject::Array(root) = root else {
        return Err(anyhow::anyhow!("Hxv4 root object is not an array"));
    };
    if root.len() % 2 != 0 {
        return Err(anyhow::anyhow!("Hxv4 root array has odd pair count"));
    }

    let mut out = Vec::new();
    for pair in root.chunks_exact(2) {
        let HxObject::Bytes(path_hash) = &pair[0] else {
            return Err(anyhow::anyhow!("Hxv4 path hash is not an octet value"));
        };
        if path_hash.len() != 8 {
            return Err(anyhow::anyhow!(format!(
                "Hxv4 path hash has {} bytes, expected 8",
                path_hash.len()
            )));
        }
        let HxObject::Array(dir) = &pair[1] else {
            return Err(anyhow::anyhow!("Hxv4 path group is not an array"));
        };
        if dir.len() % 2 != 0 {
            return Err(anyhow::anyhow!("Hxv4 path group has odd pair count"));
        }

        let mut path = [0u8; 8];
        path.copy_from_slice(path_hash);
        for entry_pair in dir.chunks_exact(2) {
            let HxObject::Bytes(name_hash) = &entry_pair[0] else {
                return Err(anyhow::anyhow!("Hxv4 filename hash is not an octet value"));
            };
            if name_hash.len() != 32 {
                return Err(anyhow::anyhow!(format!(
                    "Hxv4 filename hash has {} bytes, expected 32",
                    name_hash.len()
                )));
            }
            let HxObject::Array(info) = &entry_pair[1] else {
                return Err(anyhow::anyhow!("Hxv4 record metadata is not an array"));
            };
            if info.len() != 2 {
                return Err(anyhow::anyhow!(format!(
                    "Hxv4 record metadata has {} values, expected 2",
                    info.len()
                )));
            }
            let HxObject::Int(packed) = &info[0] else {
                return Err(anyhow::anyhow!("Hxv4 packed locator is not an integer"));
            };
            let HxObject::Int(entry_key) = &info[1] else {
                return Err(anyhow::anyhow!("Hxv4 entry key is not an integer"));
            };
            if *packed < 0 {
                return Err(anyhow::anyhow!("Hxv4 packed locator is negative"));
            }

            let packed = *packed as u64;
            // `sub_10005850` proves the 64-bit layout: EAX (low32) is fed to
            // `sub_100179A0` to construct the fake XP3 name; EDX (high32) is
            // split into archive_slot=high16 and the per-entry local value=low16.
            let id = packed & 0xffff_ffff;
            let native_locator = (packed >> 32) as u32;
            let archive_slot = (native_locator >> 16) as u16;
            let filter_flag = native_locator as u16;
            let mut name = [0u8; 32];
            name.copy_from_slice(name_hash);
            out.push(Hxv4IndexEntry {
                record_index: out.len(),
                packed,
                archive_slot,
                filter_flag,
                id,
                entry_key: *entry_key as u64,
                path_hash: path,
                name_hash: name,
                path: None,
                name: None,
            });
        }
    }

    if out.is_empty() {
        return Err(anyhow::anyhow!("Hxv4 index contains no records"));
    }
    out.sort_by_key(|entry| (entry.archive_slot, entry.id, entry.record_index));
    Ok(out)
}

/// Directory hash under the default media name (shared engine implementation).
pub fn hxv4_path_hash(path: &str) -> [u8; 8] {
    crysirror_resource::compound::path_hash(path, HXV4_SALT)
}
/// Filename hash under the default media name (shared engine implementation).
pub fn hxv4_filename_hash(name: &str) -> [u8; 32] {
    crysirror_resource::compound::file_hash(name, HXV4_SALT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chacha20poly1305::aead::Aead;
    fn seal(plain: &[u8], keys: &Hxv4IndexKeys) -> Vec<u8> {
        let sealed = XChaCha20Poly1305::new_from_slice(&keys.key)
            .unwrap()
            .encrypt(XNonce::from_slice(&keys.nonce), plain)
            .unwrap();
        [
            sealed[sealed.len() - 16..].to_vec(),
            sealed[..sealed.len() - 16].to_vec(),
        ]
        .concat()
    }
    #[test]
    fn authenticated_index_rejects_tampering_and_bounded_inflate() {
        let keys = Hxv4IndexKeys {
            key: [7; 32],
            nonce: [8; 24],
        };
        let data = b"synthetic";
        let blob = seal(data, &keys);
        assert_eq!(decrypt_hxv4_special_plaintext(&blob, &keys).unwrap(), data);
        let mut bad = blob;
        bad[0] ^= 1;
        assert!(decrypt_hxv4_special_plaintext(&bad, &keys).is_err());
        let huge = seal(
            &(16 * 1024 * 1024u32 + 1)
                .to_le_bytes()
                .iter()
                .copied()
                .chain([0])
                .collect::<Vec<_>>(),
            &keys,
        );
        assert!(decrypt_hxv4_special_payload(&huge, &keys, 16 * 1024 * 1024).is_err());
        assert!(
            decrypt_hxv4_special_index(&seal(&[0; 21], &keys), &keys, 16 * 1024 * 1024).is_err()
        );
    }
    #[test]
    fn malformed_hx_objects_fail_before_overallocating() {
        let mut c = Cursor::new(&[0x81, 0x7f, 0xff, 0xff, 0xff]);
        assert!(read_hx_object(&mut c, 0).is_err());
        let mut c = Cursor::new(&[0x02, 0xff, 0xff, 0xff, 0xff]);
        assert!(read_hx_object(&mut c, 0).is_err());
    }
}
