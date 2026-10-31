// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Constant-byte filter derived from the folded entry Adler-32, with an optional
//! separately authenticated zero-key substitution. No title/module/key catalog.
//! The generic algorithm is documented by GARbro b09ee4570ccb1daf6ac56710ee8934dc0b8baeb0
//! ArcFormats/KiriKiri/CryptAlgorithms.cs (YuzuCrypt, MIT); copyright morkt,
//! notice retained in docs/third-party/garbro-kirikiri.txt. Here parameters are
//! inferred using the shared checksum solver, then checked on every entry open.
use super::{cxdec::Adler, *};
use crate::common::{EntryPlan, ParsedArchive};
use crysirror_resource::xor_recovery::{
    single_byte_xor_adler32_candidates, MAX_XOR_RECOVERY_BYTES,
};
use std::collections::BTreeSet;
const SAMPLE_BYTES: u64 = MAX_XOR_RECOVERY_BYTES as u64;
const MAX_SAMPLES: usize = 32;

pub(super) fn requested(ctx: &DecodeContext) -> bool {
    ctx.parameters
        .get("kirikiri/xp3")
        .and_then(|v| v.get("hash_fold_xor"))
        .is_some()
}
fn fold(hash: u32) -> u8 {
    (hash ^ (hash >> 8) ^ (hash >> 16) ^ (hash >> 24)) as u8
}
#[cfg(test)]
fn checksum(input: &[u8]) -> u32 {
    let mut adler = Adler::default();
    adler.update(input);
    adler.finish()
}
fn hash(entry: &ArchiveEntry) -> Result<u32> {
    u32::try_from(
        entry.metadata["adler32"]
            .as_u64()
            .context("XP3 adlr absent")?,
    )
    .context("XP3 adlr range")
}
#[derive(Clone, Copy)]
struct Profile {
    bias: u8,
    zero: Option<u8>,
}
fn byte(value: &serde_json::Value, name: &str) -> Result<Option<u8>> {
    value
        .get(name)
        .map(|v| {
            u8::try_from(
                v.as_u64()
                    .context("hash-fold XOR parameter must be a byte")?,
            )
            .context("hash-fold XOR parameter range")
        })
        .transpose()
}
fn recover_sample(
    base: &ParsedArchive,
    entry: &ArchiveEntry,
    ctx: &DecodeContext,
) -> Result<Vec<u8>> {
    let raw = base.open(entry.id, ctx)?;
    let mut candidates = Vec::new();
    // Reserve the temporary read before allocating. Nothing is retained after
    // inference, and neither source bytes nor a partial result are published.
    let temporary = ctx.decode_with(entry.size, || {
        let data = read_all(raw.as_ref(), SAMPLE_BYTES)?;
        candidates =
            single_byte_xor_adler32_candidates(&data, hash(entry)?, &|| ctx.check_cancelled())?;
        Ok(data)
    })?;
    drop(temporary);
    Ok(candidates)
}
fn infer(samples: &[(u32, Vec<u8>)]) -> Option<Profile> {
    let mut profiles = Vec::new();
    for bias in 0..=255u8 {
        let mut hashes = BTreeSet::new();
        let mut zeros: Option<BTreeSet<u8>> = None;
        let valid = samples.iter().all(|(hash, keys)| {
            let key = fold(*hash) ^ bias;
            if key != 0 {
                hashes.insert(*hash);
                keys.contains(&key)
            } else {
                let keys: BTreeSet<_> = keys.iter().copied().collect();
                zeros = Some(zeros.take().map_or(keys.clone(), |old| {
                    old.intersection(&keys).copied().collect()
                }));
                !zeros.as_ref().unwrap().is_empty()
            }
        });
        if valid && hashes.len() >= 2 {
            let zero = zeros.and_then(|s| (s.len() == 1).then(|| *s.first().unwrap()));
            profiles.push(Profile { bias, zero });
        }
    }
    (profiles.len() == 1).then(|| profiles[0])
}
pub(super) fn open(mut base: ParsedArchive, ctx: &DecodeContext) -> Result<Box<dyn Archive>> {
    let parameter = &ctx.parameters["kirikiri/xp3"]["hash_fold_xor"];
    ensure!(
        parameter.is_object(),
        "hash_fold_xor must be a profile object"
    );
    let bias = byte(parameter, "key_bias")?;
    let zero = byte(parameter, "zero_key")?;
    ensure!(
        bias.is_some() || zero.is_none(),
        "hash-fold zero key without a bias"
    );
    let mut protected = BTreeSet::new();
    for (entry, plan) in base.entries.iter().zip(base.plans.iter_mut()) {
        if let EntryPlan::Segments { encrypted, .. } = plan {
            if *encrypted {
                protected.insert(entry.id);
            }
            *encrypted = false;
        }
    }
    let mut ordered: Vec<_> = base
        .entries
        .iter()
        .filter(|e| protected.contains(&e.id))
        .collect();
    ordered.sort_by_key(|e| e.size);
    let mut bytes = 0u64;
    let mut profile = if let Some(bias) = bias {
        Profile { bias, zero }
    } else {
        let mut samples = Vec::new();
        let mut recovered = None;
        for entry in ordered.iter().take(MAX_SAMPLES) {
            ctx.check_cancelled()?;
            if entry.size > SAMPLE_BYTES.saturating_sub(bytes) {
                break;
            }
            bytes += entry.size;
            let keys = recover_sample(&base, entry, ctx)?;
            ensure!(
                !keys.is_empty(),
                "hash-fold XOR has no constant-byte checksum solution: {}",
                entry.path
            );
            if keys.contains(&0) {
                continue;
            } // Authenticated plaintext bootstrap, no cipher evidence.
            samples.push((hash(entry)?, keys));
            if let Some(p) = infer(&samples) {
                recovered = Some(p);
                break;
            }
        }
        recovered.context(
            "hash-fold XOR needs a unique bias supported by two distinct protected entry hashes",
        )?
    };
    if profile.zero.is_none() {
        let bias = profile.bias;
        for entry in ordered
            .iter()
            .filter(|e| hash(e).is_ok_and(|h| fold(h) == bias))
            .take(MAX_SAMPLES)
        {
            ctx.check_cancelled()?;
            if entry.size > SAMPLE_BYTES.saturating_sub(bytes) {
                continue;
            }
            bytes += entry.size;
            let keys = recover_sample(&base, entry, ctx)?;
            ensure!(
                !keys.is_empty(),
                "hash-fold XOR zero-domain checksum mismatch: {}",
                entry.path
            );
            if keys.contains(&0) {
                continue;
            }
            ensure!(keys.len() == 1, "ambiguous hash-fold XOR zero-domain key");
            profile.zero = Some(keys[0]);
            break;
        }
    }
    for entry in base
        .entries
        .iter_mut()
        .filter(|e| protected.contains(&e.id))
    {
        entry.metadata["hash_fold_xor"] =
            serde_json::json!({"key_bias":profile.bias,"zero_key":profile.zero});
    }
    let archive = FoldArchive {
        base,
        protected,
        profile,
    };
    // Validate each selected archive independently, including when a previous
    // archive supplied parameters. This cannot replace per-entry verification.
    if let Some(entry) = archive
        .base
        .entries
        .iter()
        .filter(|e| archive.protected.contains(&e.id))
        .min_by_key(|e| e.size)
    {
        archive.open(entry.id, ctx)?;
    }
    Ok(Box::new(archive))
}
struct FoldArchive {
    base: ParsedArchive,
    protected: BTreeSet<u32>,
    profile: Profile,
}
impl Archive for FoldArchive {
    fn entries(&self) -> &[ArchiveEntry] {
        &self.base.entries
    }
    fn open(&self, id: u32, ctx: &DecodeContext) -> Result<Arc<dyn AssetReader>> {
        ctx.check_cancelled()?;
        let raw = self.base.open(id, ctx)?;
        if !self.protected.contains(&id) {
            return Ok(raw);
        }
        let entry = self
            .base
            .entries
            .get(id as usize)
            .context("hash-fold entry id")?;
        let expected = hash(entry)?;
        let folded = fold(expected) ^ self.profile.bias;
        let key = if folded != 0 {
            Some(folded)
        } else {
            self.profile.zero
        };
        let mut plain = Adler::default();
        let mut decoded = Adler::default();
        let mut offset = 0u64;
        let mut buf = [0u8; 65536];
        while offset < entry.size {
            ctx.check_cancelled()?;
            let n = (entry.size - offset).min(buf.len() as u64) as usize;
            crysirror_resource::read_exact_at(raw.as_ref(), offset, &mut buf[..n])?;
            plain.update(&buf[..n]);
            if let Some(key) = key {
                for b in &mut buf[..n] {
                    *b ^= key;
                }
                decoded.update(&buf[..n]);
            }
            offset += n as u64;
        }
        ctx.check_cancelled()?;
        if plain.finish() == expected {
            return Ok(raw);
        }
        let key = key.context("hash-fold XOR zero-domain key is unproven")?;
        ensure!(
            decoded.finish() == expected,
            "hash-fold XOR decoded Adler-32 mismatch: {}",
            entry.path
        );
        Ok(Arc::new(FilterReader { source: raw, key }))
    }
}
struct FilterReader {
    source: Arc<dyn AssetReader>,
    key: u8,
}
impl AssetReader for FilterReader {
    fn len(&self) -> crysirror_core::Result<u64> {
        self.source.len()
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        let n = self.source.read_at(offset, out)?;
        if n > out.len() {
            return Err(crysirror_core::Error::Invalid("hash-fold reader count"));
        }
        for b in &mut out[..n] {
            *b ^= self.key;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::{service::MemoryAssetReader, state::ClientId};
    fn fixture(corrupt: bool) -> (ParsedArchive, DecodeContext, Vec<Vec<u8>>) {
        let mut zero = vec![0x3a; 32];
        let mut found = false;
        for n in 0..=65535u32 {
            zero[30..].copy_from_slice(&(n as u16).to_le_bytes());
            if fold(checksum(&zero)) == 0xd2 {
                found = true;
                break;
            }
        }
        assert!(found);
        let plains = vec![
            b"TJS2100\0author bootstrap".to_vec(),
            (0..100).map(|n| (n * 23) as u8).collect(),
            (0..117).map(|n| (n * 41 + 3) as u8).collect(),
            zero,
            (0..401).map(|n| (n * 11 + 7) as u8).collect(),
        ];
        let mut source = Vec::new();
        let mut entries = Vec::new();
        let mut plans = Vec::new();
        for (id, plain) in plains.iter().enumerate() {
            let hash = checksum(plain);
            let folded = fold(hash) ^ 0xd2;
            let key = if id == 0 {
                0
            } else if folded == 0 {
                0xd0
            } else {
                folded
            };
            let offset = source.len() as u64;
            let mut encoded: Vec<_> = plain.iter().map(|b| b ^ key).collect();
            if corrupt && id == 4 {
                encoded[107] ^= 0x80;
            }
            source.extend(encoded);
            entries.push(ArchiveEntry {
                id: id as u32,
                path: format!("resource/{id}"),
                size: plain.len() as u64,
                stored_size: plain.len() as u64,
                metadata: serde_json::json!({"adler32":hash}),
            });
            plans.push(EntryPlan::Segments {
                segments: vec![Segment {
                    offset,
                    stored: plain.len() as u64,
                    size: plain.len() as u64,
                    compressed: false,
                }],
                encrypted: true,
            });
        }
        let mut ctx = DecodeContext::new(ClientId(71));
        ctx.parameters.insert(
            "kirikiri/xp3".into(),
            serde_json::json!({"hash_fold_xor":{}}),
        );
        (
            ParsedArchive {
                format: "kirikiri/xp3",
                source: Arc::new(MemoryAssetReader(source.into())),
                entries,
                plans,
            },
            ctx,
            plains,
        )
    }
    #[test]
    fn recovers_bias_zero_substitution_and_authenticates_plain_bootstrap_and_random_reads() {
        let (base, ctx, plain) = fixture(false);
        let archive = open(base, &ctx).unwrap();
        assert_eq!(
            archive.entries()[0].metadata["hash_fold_xor"],
            serde_json::json!({"key_bias":210,"zero_key":208})
        );
        assert_eq!(ctx.resident_bytes(), 0);
        for (id, expected) in plain.iter().enumerate() {
            let reader = archive.open(id as u32, &ctx).unwrap();
            let mut part = [0u8; 7];
            crysirror_resource::read_exact_at(reader.as_ref(), 3, &mut part).unwrap();
            assert_eq!(&part, &expected[3..10]);
            assert_eq!(read_all(reader.as_ref(), 1000).unwrap(), *expected);
        }
        assert_eq!(ctx.resident_bytes(), 0);
    }
    #[test]
    fn later_corruption_and_incomplete_evidence_are_errors() {
        let (base, ctx, _) = fixture(true);
        let archive = open(base, &ctx).unwrap();
        assert!(archive
            .open(4, &ctx)
            .err()
            .unwrap()
            .to_string()
            .contains("Adler-32"));
        let (mut base, ctx, _) = fixture(false);
        base.entries.truncate(2);
        base.plans.truncate(2);
        assert!(open(base, &ctx)
            .err()
            .unwrap()
            .to_string()
            .contains("two distinct"));
    }
    #[test]
    fn unproved_zero_substitution_and_wrong_profile_cannot_publish_bytes() {
        let (mut base, ctx, _) = fixture(false);
        for plan in &mut base.plans {
            if let EntryPlan::Segments { encrypted, .. } = plan {
                *encrypted = false;
            }
        }
        let archive = FoldArchive {
            base,
            protected: (0..5).collect(),
            profile: Profile {
                bias: 0xd2,
                zero: None,
            },
        };
        assert!(archive
            .open(3, &ctx)
            .err()
            .unwrap()
            .to_string()
            .contains("unproven"));
        let (base, mut ctx, _) = fixture(false);
        ctx.parameters.insert(
            "kirikiri/xp3".into(),
            serde_json::json!({"hash_fold_xor":{"key_bias":211,"zero_key":208}}),
        );
        // A plain bootstrap can be accepted as plain, but must never authorize
        // incorrectly decrypted later resources under the same archive.
        match open(base, &ctx) {
            Ok(a) => assert!(a.open(1, &ctx).is_err()),
            Err(_) => {}
        }
    }
}
