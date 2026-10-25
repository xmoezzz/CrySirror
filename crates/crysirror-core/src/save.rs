// SPDX-License-Identifier: MPL-2.0
//! A versioned envelope for separate service and native-VM snapshots.
//! Payload codecs remain explicit and versioned. CRC detects accidental damage,
//! not hostile tampering. This is not an importer for original game save files.
use crate::{Error, Result};
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

const MAGIC: &[u8; 8] = b"CRYSAVE\0";
const VERSION: u32 = 1;
const MAX_ID_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveIdentity {
    pub engine: String,
    pub game: String,
    /// Content/build fingerprint supplied by the packager, not guessed here.
    pub content_revision: String,
    pub vm_schema: u32,
    pub services_schema: u32,
}

impl SaveIdentity {
    pub fn validate(&self) -> Result<()> {
        for s in [&self.engine, &self.game, &self.content_revision] {
            if s.is_empty() || s.len() > MAX_ID_BYTES || s.chars().any(char::is_control) {
                return Err(Error::Invalid("snapshot identity"));
            }
        }
        if self.vm_schema == 0 || self.services_schema == 0 {
            return Err(Error::Invalid("snapshot schema"));
        }
        Ok(())
    }
    pub fn compatible_with(&self, expected: &Self) -> Result<()> {
        self.validate()?;
        expected.validate()?;
        if self.engine != expected.engine {
            return Err(Error::SnapshotMismatch("engine"));
        }
        if self.game != expected.game {
            return Err(Error::SnapshotMismatch("game"));
        }
        if self.content_revision != expected.content_revision {
            return Err(Error::SnapshotMismatch("content revision"));
        }
        if self.vm_schema != expected.vm_schema {
            return Err(Error::SnapshotMismatch("VM schema"));
        }
        if self.services_schema != expected.services_schema {
            return Err(Error::SnapshotMismatch("services schema"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SaveLimits {
    pub total_bytes: usize,
    pub section_bytes: usize,
}
impl Default for SaveLimits {
    fn default() -> Self {
        Self {
            total_bytes: 64 * 1024 * 1024,
            section_bytes: 32 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveEnvelope {
    pub identity: SaveIdentity,
    pub services: Vec<u8>,
    pub vm: Vec<u8>,
}

impl SaveEnvelope {
    pub fn encode(&self, limits: SaveLimits) -> Result<Vec<u8>> {
        self.identity.validate()?;
        if self.services.len() > limits.section_bytes || self.vm.len() > limits.section_bytes {
            return Err(Error::Limit("snapshot section"));
        }
        // Check before allocating/copying, including all prefixes and the CRC.
        let len = 8usize
            .checked_add(4 + 8 + 5 * 4 + 4)
            .and_then(|n| n.checked_add(self.identity.engine.len()))
            .and_then(|n| n.checked_add(self.identity.game.len()))
            .and_then(|n| n.checked_add(self.identity.content_revision.len()))
            .and_then(|n| n.checked_add(self.services.len()))
            .and_then(|n| n.checked_add(self.vm.len()))
            .ok_or(Error::Limit("snapshot length"))?;
        if len > limits.total_bytes {
            return Err(Error::Limit("snapshot total"));
        }
        let mut out = Vec::with_capacity(len);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&self.identity.vm_schema.to_le_bytes());
        out.extend_from_slice(&self.identity.services_schema.to_le_bytes());
        for b in [
            self.identity.engine.as_bytes(),
            self.identity.game.as_bytes(),
            self.identity.content_revision.as_bytes(),
            &self.services,
            &self.vm,
        ] {
            let length =
                u32::try_from(b.len()).map_err(|_| Error::Limit("snapshot section u32"))?;
            out.extend_from_slice(&length.to_le_bytes());
            out.extend_from_slice(b);
        }
        let checksum = crc32(&out);
        out.extend_from_slice(&checksum.to_le_bytes());
        Ok(out)
    }

    pub fn decode(bytes: &[u8], expected: &SaveIdentity, limits: SaveLimits) -> Result<Self> {
        expected.validate()?;
        if bytes.len() > limits.total_bytes {
            return Err(Error::Limit("snapshot total"));
        }
        if bytes.len() < 44 {
            return Err(Error::CorruptSnapshot);
        }
        let body_len = bytes.len() - 4;
        let stored = u32::from_le_bytes(
            bytes[body_len..]
                .try_into()
                .map_err(|_| Error::CorruptSnapshot)?,
        );
        if crc32(&bytes[..body_len]) != stored {
            return Err(Error::CorruptSnapshot);
        }
        let mut reader = Reader {
            bytes: &bytes[..body_len],
            pos: 0,
        };
        if reader.take(8)? != MAGIC {
            return Err(Error::CorruptSnapshot);
        }
        if reader.u32()? != VERSION {
            return Err(Error::Unsupported("save envelope version"));
        }
        let vm_schema = reader.u32()?;
        let services_schema = reader.u32()?;
        let identity = SaveIdentity {
            engine: reader.string()?,
            game: reader.string()?,
            content_revision: reader.string()?,
            vm_schema,
            services_schema,
        };
        identity.compatible_with(expected)?;
        // Lengths and identity are checked before allocating the payloads.
        let services = reader.section(limits.section_bytes)?;
        let vm = reader.section(limits.section_bytes)?;
        if reader.pos != body_len {
            return Err(Error::CorruptSnapshot);
        }
        Ok(Self {
            identity,
            services: services.to_vec(),
            vm: vm.to_vec(),
        })
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(len).ok_or(Error::CorruptSnapshot)?;
        let result = self
            .bytes
            .get(self.pos..end)
            .ok_or(Error::CorruptSnapshot)?;
        self.pos = end;
        Ok(result)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| Error::CorruptSnapshot)?,
        ))
    }
    fn section(&mut self, max: usize) -> Result<&'a [u8]> {
        let len = usize::try_from(self.u32()?).map_err(|_| Error::CorruptSnapshot)?;
        if len > max {
            return Err(Error::Limit("snapshot section"));
        }
        self.take(len)
    }
    fn string(&mut self) -> Result<String> {
        let s = core::str::from_utf8(self.section(MAX_ID_BYTES)?)
            .map_err(|_| Error::CorruptSnapshot)?;
        Ok(s.to_string())
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for b in bytes {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    fn fixture() -> SaveEnvelope {
        SaveEnvelope {
            identity: SaveIdentity {
                engine: "rfvp".into(),
                game: "synthetic".into(),
                content_revision: "fixture-v1".into(),
                vm_schema: 1,
                services_schema: 1,
            },
            services: vec![1, 2, 3],
            vm: vec![4, 5, 6],
        }
    }
    #[test]
    fn round_trip() {
        let s = fixture();
        let b = s.encode(SaveLimits::default()).unwrap();
        assert_eq!(
            SaveEnvelope::decode(&b, &s.identity, SaveLimits::default()).unwrap(),
            s
        );
    }
    #[test]
    fn crc_reference_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf43926);
    }
    #[test]
    fn rejects_every_truncation() {
        let s = fixture();
        let b = s.encode(SaveLimits::default()).unwrap();
        for end in 0..b.len() {
            assert!(SaveEnvelope::decode(&b[..end], &s.identity, SaveLimits::default()).is_err());
        }
    }
    #[test]
    fn rejects_bit_flips() {
        let s = fixture();
        let original = s.encode(SaveLimits::default()).unwrap();
        for i in 0..original.len() {
            let mut b = original.clone();
            b[i] ^= 1;
            assert!(SaveEnvelope::decode(&b, &s.identity, SaveLimits::default()).is_err());
        }
    }
    #[test]
    fn rejects_cross_engine_saves() {
        let s = fixture();
        let b = s.encode(SaveLimits::default()).unwrap();
        let mut expected = s.identity.clone();
        expected.engine = "siglus".into();
        assert_eq!(
            SaveEnvelope::decode(&b, &expected, SaveLimits::default()),
            Err(Error::SnapshotMismatch("engine"))
        );
    }
    #[test]
    fn bounds_are_enforced() {
        let s = fixture();
        assert!(s
            .encode(SaveLimits {
                total_bytes: 10,
                section_bytes: 20
            })
            .is_err());
        assert!(s
            .encode(SaveLimits {
                total_bytes: 200,
                section_bytes: 2
            })
            .is_err());
    }
}
