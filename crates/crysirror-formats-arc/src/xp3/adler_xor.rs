// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! XP3's 31-byte Adler-derived rotating XOR filter. The companion is only
//! statically inspected as PE data; no DLL loading or instruction emulation.
//! This recognizes a bounded, known compiler shape and verifies every decoded
//! entry against its archive checksum before publishing a random-access reader.
use super::{
    cxdec::{pe, Adler},
    *,
};
use crate::common::{EntryPlan, ParsedArchive};
use std::collections::BTreeSet;

// Instruction semantics, not a title/module fingerprint: hash at info+20;
// duplicate its low bit into bit31; emit 31 low bytes while rotating a 31-bit
// state right by 8; index the key with the full 64-bit extraction offset.
const SEED: &[u8] = &[
    0x8b, 0x47, 0x14, 0x25, 0xff, 0xff, 0xff, 0x7f, 0x8b, 0xc8, 0xc1, 0xe1, 0x1f, 0x0b, 0xc1, 0x33,
    0xc9,
];
const RECURRENCE: &[u8] = &[
    0x8b, 0xd0, 0x88, 0x44, 0x0c, 0x08, 0x83, 0xe2, 0xfe, 0xc1, 0xe2, 0x17, 0xc1, 0xe8, 0x08, 0x41,
    0x0b, 0xc2, 0x83, 0xf9, 0x1f, 0x7c, 0xe9,
];
const OFFSET: &[u8] = &[
    0x8b, 0x47, 0x0c, 0x33, 0xd2, 0x8b, 0xce, 0x03, 0x4f, 0x04, 0x6a, 0, 0x13, 0x57, 0x08, 0x6a,
    0x1f, 0x52, 0x51,
];

pub(super) fn requested(ctx: &DecodeContext) -> bool {
    ctx.parameters
        .get("kirikiri/xp3")
        .and_then(|v| v.get("adler_xor_modules"))
        .is_some()
}
fn evidence(bytes: &[u8]) -> bool {
    let Ok(image) = pe::PeImage::parse(bytes) else {
        return false;
    };
    if !image.i386() {
        return false;
    }
    let found = image.exec_ranges(bytes.len()).any(|(a, b)| {
        bytes[a..b].windows(SEED.len()).enumerate().any(|(i, s)| {
            if s != SEED {
                return false;
            }
            let start = a + i;
            // The key generation and offset-index operation must belong to
            // this same short callback, rather than unrelated code sections.
            let callback = &bytes[start..(start + 128).min(b)];
            callback.windows(RECURRENCE.len()).any(|s| s == RECURRENCE)
                && callback.windows(OFFSET.len()).any(|s| s == OFFSET)
                && callback.windows(2).any(|s| s == [0x30, 0x03])
        })
    });
    found
}
fn key(hash: u32) -> [u8; 31] {
    let mut state = (hash << 31) | (hash & 0x7fff_ffff);
    std::array::from_fn(|_| {
        let byte = state as u8;
        state = ((state & 0xffff_fffe) << 23) | (state >> 8);
        byte
    })
}
fn apply(key: &[u8; 31], offset: u64, bytes: &mut [u8]) {
    // Reduce first: adding a byte index to a caller-supplied u64 must not wrap.
    let start = (offset % 31) as usize;
    for (i, b) in bytes.iter_mut().enumerate() {
        *b ^= key[(start + i % 31) % 31];
    }
}
pub(super) fn open(mut base: ParsedArchive, ctx: &DecodeContext) -> Result<Box<dyn Archive>> {
    let modules = ctx.parameters["kirikiri/xp3"]["adler_xor_modules"]
        .as_array()
        .context("adler_xor_modules must be a capability-relative companion list")?;
    ensure!(
        !modules.is_empty() && modules.len() <= 16,
        "rotating XOR needs 1..=16 companions"
    );
    let mut recognized = Vec::new();
    for module in modules {
        let name = module
            .as_str()
            .context("rotating XOR module name must be a string")?;
        let bytes = read_all(
            ctx.open_sibling(name)?.as_ref(),
            pe::MAX_MODULE_BYTES as u64,
        )?;
        if evidence(&bytes) {
            recognized.push(name.to_owned());
        }
    }
    ensure!(
        !recognized.is_empty(),
        "no supported rotating Adler XOR callback evidence in supplied modules"
    );
    let mut protected = BTreeSet::new();
    for (entry, plan) in base.entries.iter_mut().zip(base.plans.iter_mut()) {
        if let EntryPlan::Segments { encrypted, .. } = plan {
            if *encrypted {
                protected.insert(entry.id);
                entry.metadata["adler_xor"] = serde_json::json!({"period":31,"modules":recognized});
            }
            *encrypted = false;
        }
    }
    let archive = AdlerArchive { base, protected };
    // Prove the selected family restores actual protected data before the
    // archive is accepted. Each subsequent open independently checks adlr.
    let sample = archive
        .protected
        .iter()
        .filter_map(|id| archive.base.entries.get(*id as usize))
        .min_by_key(|e| e.size);
    if let Some(entry) = sample {
        archive.open(entry.id, ctx)?;
    }
    Ok(Box::new(archive))
}
struct AdlerArchive {
    base: ParsedArchive,
    protected: BTreeSet<u32>,
}
impl Archive for AdlerArchive {
    fn entries(&self) -> &[ArchiveEntry] {
        &self.base.entries
    }
    fn open(&self, id: u32, ctx: &DecodeContext) -> Result<Arc<dyn AssetReader>> {
        let raw = self.base.open(id, ctx)?;
        if !self.protected.contains(&id) {
            return Ok(raw);
        }
        let entry = self
            .base
            .entries
            .get(id as usize)
            .context("rotating XOR entry id")?;
        let hash = entry.metadata["adler32"]
            .as_u64()
            .context("XP3 adlr absent")? as u32;
        let reader = FilterReader {
            source: raw,
            key: key(hash),
        };
        let mut checksum = Adler::default();
        let mut offset = 0u64;
        let mut chunk = [0u8; 65536];
        while offset < entry.size {
            let n = (entry.size - offset).min(chunk.len() as u64) as usize;
            crysirror_resource::read_exact_at(&reader, offset, &mut chunk[..n])?;
            checksum.update(&chunk[..n]);
            offset += n as u64;
        }
        ensure!(
            checksum.finish() == hash,
            "rotating Adler XOR decoded entry checksum mismatch: {}",
            entry.path
        );
        Ok(Arc::new(reader))
    }
}
struct FilterReader {
    source: Arc<dyn AssetReader>,
    key: [u8; 31],
}
impl AssetReader for FilterReader {
    fn len(&self) -> crysirror_core::Result<u64> {
        self.source.len()
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        let n = self.source.read_at(offset, out)?;
        if n > out.len() {
            return Err(crysirror_core::Error::Invalid("rotating XOR reader count"));
        }
        apply(&self.key, offset, &mut out[..n]);
        Ok(n)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_adler_vector_and_large_random_access_offset() {
        // Independent hand-calculated first eight bytes for the authored hash 0x12345678.
        assert_eq!(
            &key(0x1234_5678)[..8],
            &[0x78, 0x56, 0x34, 0x12, 0x3c, 0x2b, 0x1a, 0x09]
        );
        let key = key(0x12345678);
        let plain: Vec<u8> = (0..1000).map(|i| (i * 17) as u8).collect();
        let mut encoded = plain.clone();
        apply(&key, u64::MAX - 5, &mut encoded);
        let mut restored = encoded[23..301].to_vec();
        apply(&key, ((u64::MAX - 5) % 31 + 23) % 31, &mut restored);
        assert_eq!(restored, plain[23..301]);
    }
    fn module() -> Vec<u8> {
        // Authored minimal PE section with this public filter's instruction
        // semantics. No installed executable bytes are stored in the test.
        let mut b = vec![0u8; 1024];
        b[..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x84..0x86].copy_from_slice(&0x14cu16.to_le_bytes());
        b[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
        b[0x94..0x96].copy_from_slice(&0xe0u16.to_le_bytes());
        b[0x98..0x9a].copy_from_slice(&0x10bu16.to_le_bytes());
        let section = 0x178;
        for (offset, value) in [
            (8, 512u32),
            (12, 0x1000),
            (16, 512),
            (20, 512),
            (36, pe::SCN_EXECUTE),
        ] {
            b[section + offset..section + offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        let mut at = 512;
        for code in [SEED, RECURRENCE, OFFSET, &[0x30, 0x03]] {
            b[at..at + code.len()].copy_from_slice(code);
            at += code.len();
        }
        b
    }
    fn fixture(corrupt: bool) -> (ParsedArchive, DecodeContext, Vec<u8>) {
        use crysirror_core::{service::MemoryAssetReader, state::ClientId};
        let plain: Vec<u8> = (0..1200).map(|n| (n * 23) as u8).collect();
        let mut checksum = Adler::default();
        checksum.update(&plain);
        let hash = checksum.finish();
        let mut stored = plain.clone();
        apply(&key(hash), 0, &mut stored);
        if corrupt {
            stored[43] ^= 1;
        }
        let base = ParsedArchive {
            format: "kirikiri/xp3",
            source: Arc::new(MemoryAssetReader(stored.clone().into())),
            entries: vec![ArchiveEntry {
                id: 0,
                path: "test.bin".into(),
                size: plain.len() as u64,
                stored_size: stored.len() as u64,
                metadata: serde_json::json!({"adler32":hash}),
            }],
            plans: vec![EntryPlan::Segments {
                segments: vec![crate::common::Segment {
                    offset: 0,
                    stored: stored.len() as u64,
                    size: stored.len() as u64,
                    compressed: false,
                }],
                encrypted: true,
            }],
        };
        let mut ctx = DecodeContext::new(ClientId(3));
        ctx.source_name = Some("data.xp3".into());
        ctx.resolver = Some(Arc::new(|path: &str| -> Result<Arc<dyn AssetReader>> {
            ensure!(path == "plugin/filter.tpm", "outside fixture capability");
            Ok(Arc::new(MemoryAssetReader(module().into())))
        }));
        ctx.parameters.insert(
            "kirikiri/xp3".into(),
            serde_json::json!({"adler_xor_modules":["plugin/filter.tpm"]}),
        );
        (base, ctx, plain)
    }
    #[test]
    fn companion_evidence_checksum_and_random_reads_are_required() {
        assert!(evidence(&module()));
        let (base, ctx, plain) = fixture(false);
        let archive = open(base, &ctx).unwrap();
        let reader = archive.open(0, &ctx).unwrap();
        let mut out = [0u8; 71];
        reader.read_at(59, &mut out).unwrap();
        assert_eq!(&out, &plain[59..130]);
        let (base, ctx, _) = fixture(true);
        assert!(open(base, &ctx)
            .err()
            .unwrap()
            .to_string()
            .contains("checksum mismatch"));
    }
    #[test]
    fn arbitrary_non_pe_and_partial_opcode_evidence_are_rejected() {
        assert!(!evidence(SEED));
        let mut bytes = vec![0; 1024];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[512..512 + SEED.len()].copy_from_slice(SEED);
        assert!(!evidence(&bytes));
    }
}
