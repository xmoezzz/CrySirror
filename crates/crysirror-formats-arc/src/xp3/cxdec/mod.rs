// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Shared CXDEC XP3 content filter and indirect Special filename index.
//!
//! Parameters are recovered at open time from the caller's explicit,
//! capability-relative companion modules (`kirikiri/xp3.cxdec_modules`, the
//! game's EXE/DLL/TPM) and, for the Special index, from the archive's own
//! startup script. No module code runs and no per-title value is embedded.
//! A profile is accepted only after it restores real entries' `adlr`; every
//! protected entry is checksum-verified again before it is published.
//! Unknown/unsupported layouts fail visibly.
mod engine;
pub(super) mod pe;
mod recover;
mod special;
use super::*;
use crate::common::{EntryPlan, ParsedArchive};
use anyhow::bail;
use engine::CxdecEngine;
use recover::ModuleFacts;
use std::collections::BTreeSet;
use std::sync::{Mutex, OnceLock};

const MAX_MODULES: usize = 16;
const SAMPLE_COUNT: usize = 32;
const SAMPLE_BYTES: u64 = 64 * 1024 * 1024;

/// Indirect Special descriptor: `offset u64, original u32, stored u32`.
pub(super) struct Special {
    tag: [u8; 4],
    offset: u64,
    original: u32,
    stored: u32,
}
pub(super) fn special_descriptor(tag: &[u8], body: &[u8], file: u64) -> Option<Special> {
    if body.len() != 16 {
        return None;
    }
    let offset = u64::from_le_bytes(body[..8].try_into().ok()?);
    let original = u32::from_le_bytes(body[8..12].try_into().ok()?);
    let stored = u32::from_le_bytes(body[12..16].try_into().ok()?);
    let tag: [u8; 4] = tag.try_into().ok()?;
    (original > 0 && stored > 0 && range(file, offset, u64::from(stored)).is_ok()).then_some(
        Special {
            tag,
            offset,
            original,
            stored,
        },
    )
}

pub(super) fn requested(ctx: &DecodeContext) -> bool {
    ctx.parameters
        .get("kirikiri/xp3")
        .and_then(|o| o.get("cxdec_modules"))
        .is_some()
}

pub(super) fn open(
    mut base: ParsedArchive,
    specials: Vec<Special>,
    ctx: &DecodeContext,
) -> Result<Box<dyn Archive>> {
    let modules = ctx
        .parameters
        .get("kirikiri/xp3")
        .and_then(|o| o.get("cxdec_modules"))
        .and_then(|v| v.as_array())
        .context("CXDEC needs kirikiri/xp3.cxdec_modules: capability-relative EXE/DLL/TPM names")?;
    ensure!(
        !modules.is_empty() && modules.len() <= MAX_MODULES,
        "CXDEC module list must name 1..=16 companions"
    );
    let mut facts = Vec::new();
    for m in modules {
        let name = m.as_str().context("CXDEC module name must be a string")?;
        let bytes = read_all(
            ctx.open_sibling(name)?.as_ref(),
            pe::MAX_MODULE_BYTES as u64,
        )?;
        facts.extend(analyzed(name, &bytes).iter().cloned());
    }
    ensure!(
        !facts.is_empty(),
        "no CXDEC generator evidence in the supplied modules"
    );
    let mut derived = None;
    let mut special_report = serde_json::Value::Null;
    let mut setup = None;
    match specials.len() {
        0 => {}
        1 => {
            let (d, report, s) = apply_special_names(&mut base, &specials[0], &facts, ctx)?;
            derived = Some(d);
            special_report = report;
            setup = Some(s);
        }
        _ => bail!("multiple XP3 indirect Special descriptors"),
    }
    // Content bytes are filtered here, never by the generic pipeline.
    let mut protected = BTreeSet::new();
    for (e, plan) in base.entries.iter().zip(base.plans.iter_mut()) {
        if let EntryPlan::Segments { encrypted, .. } = plan {
            if *encrypted {
                protected.insert(e.id);
            }
            *encrypted = false;
        }
    }
    let candidates = match &derived {
        Some(d) => {
            let c = recover::coherent_candidates(&facts, &d.control_block, (d.mask, d.offset));
            if c.is_empty() {
                let mut extended = facts.clone();
                extended.push(ModuleFacts {
                    label: "setupArchiveData".into(),
                    mask_offset: Some((d.mask, d.offset)),
                    control_blocks: vec![d.control_block.clone()],
                    ..Default::default()
                });
                recover::generic_candidates(&extended)
            } else {
                c
            }
        }
        None => recover::generic_candidates(&facts),
    };
    let samples = samples(&base, &protected, ctx)?;
    let engine = select(candidates, &samples)?;
    let engine_report = engine.as_ref().map(|(e, src)| {
        let p = e.profile();
        serde_json::json!({"generator": match p.generator {
            engine::GeneratorKind::Classic => "classic",
            engine::GeneratorKind::Cabbage{..} => "cabbage",
        }, "prefix8": p.prefix8.is_some(), "source": src, "validated_samples": samples.len()})
    });
    for e in &mut base.entries {
        if protected.contains(&e.id) {
            e.metadata["cxdec"] =
                serde_json::json!({"special": special_report, "content": engine_report});
        }
    }
    let mut embedded = BTreeMap::new();
    let target = ctx
        .parameters
        .get("kirikiri/xp3")
        .and_then(|o| o.get("cxdec_bootstrap_profile"));
    if let Some(target) = target {
        let target = target
            .as_str()
            .context("cxdec_bootstrap_profile must be a path string")?;
        // Only the archive holding the native startup redirect describes the
        // bootstrap; sibling containers validated by the same text do not.
        if let Some(setup) = setup.as_ref().filter(|s| s.startup_archive) {
            let data = bootstrap_profile(setup, &special_report, &engine_report)?;
            let path =
                crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, target)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            ensure!(
                path.relative() == target && target.len() <= 512,
                "CXDEC bootstrap profile target must be canonical"
            );
            ensure!(
                base.entries.len() < ctx.limits.max_entries
                    && !base.entries.iter().any(|e| e.path == target),
                "CXDEC bootstrap profile target collision or count limit"
            );
            let id = base.entries.len() as u32;
            base.entries.push(ArchiveEntry {
                id,
                path: target.into(),
                size: data.len() as u64,
                stored_size: 0,
                metadata: serde_json::json!({"format": "kirikiri/xp3",
                    "origin": "validated CXDEC setupArchiveData profile", "derived": true}),
            });
            embedded.insert(id, data);
        }
    }
    Ok(Box::new(CxArchive {
        base,
        protected,
        engine: engine.map(|(e, _)| Arc::new(e)),
        embedded,
    }))
}

/// The `Storages.setupArchiveData` argument that authenticated this archive's
/// Special index (its derived seeds decrypted and fully mapped the records).
struct SetupData {
    text: String,
    startup_archive: bool,
}

/// Engine-owned description of the validated CXDEC bootstrap consumed by the
/// native `Storages.setupArchiveData` facade. It names the exact argument the
/// mount accepted so a native call can be checked against it; it grants no
/// other authority and is never written outside the mounted container.
fn bootstrap_profile(
    setup: &SetupData,
    special: &serde_json::Value,
    content: &Option<serde_json::Value>,
) -> Result<Arc<[u8]>> {
    let profile = serde_json::json!({
        "family": "cxdec-setup-archive-data",
        "setup_archive_data": setup.text,
        "setup_archive_data_units": setup.text.encode_utf16().count(),
        "startup": special["startup"],
        "records": special["records"],
        "content": content,
    });
    Ok(Arc::from(serde_json::to_vec_pretty(&profile)?))
}

/// Deterministic static analysis memoized by module digest.
fn analyzed(name: &str, bytes: &[u8]) -> Arc<Vec<ModuleFacts>> {
    use sha2::{Digest, Sha256};
    static CACHE: OnceLock<Mutex<Vec<([u8; 32], Arc<Vec<ModuleFacts>>)>>> = OnceLock::new();
    let key: [u8; 32] = Sha256::digest(bytes).into();
    let cache = CACHE.get_or_init(Default::default);
    if let Some((_, f)) = cache.lock().unwrap().iter().find(|(k, _)| *k == key) {
        return f.clone();
    }
    let facts = Arc::new(recover::analyze(name, bytes));
    let mut c = cache.lock().unwrap();
    if c.len() >= 16 {
        c.remove(0);
    }
    c.push((key, facts.clone()));
    facts
}

/// Recover and authenticate filenames, rewriting entry paths in place.
fn apply_special_names(
    base: &mut ParsedArchive,
    d: &Special,
    facts: &[ModuleFacts],
    ctx: &DecodeContext,
) -> Result<(special::Derived, serde_json::Value, SetupData)> {
    let redirects: BTreeSet<(String, String)> = facts
        .iter()
        .filter(|f| f.setup_generator)
        .filter_map(|f| f.startup_redirect.clone())
        .collect();
    ensure!(
        facts.iter().any(|f| f.setup_generator),
        "XP3 Special index: no setupArchiveData generator in the supplied modules"
    );
    ensure!(
        redirects.len() == 1,
        "XP3 Special index: startup redirect absent or ambiguous ({} found)",
        redirects.len()
    );
    let (archive_name, member) = redirects.into_iter().next().unwrap();
    let own = ctx
        .source_name
        .as_deref()
        .map(|s| {
            s.rsplit('/')
                .next()
                .unwrap_or(s)
                .eq_ignore_ascii_case(&archive_name)
        })
        .unwrap_or(false);
    let script = if own {
        plain_member(base, &member, ctx)?
    } else {
        let source = ctx.open_sibling(&archive_name)?;
        let index = parse_index(source, ctx)?;
        plain_member(&index.archive, &member, ctx)?
    };
    let blob = bytes(
        base.source.as_ref(),
        d.offset,
        u64::from(d.stored),
        ctx.limits.max_index_size,
    )?;
    let prefix = std::str::from_utf8(&d.tag).context("Special descriptor tag is not text")?;
    let mut tried = 0usize;
    for text in special::script_strings(&script).into_iter().take(4096) {
        let Ok(derived) = special::derive(&text) else {
            continue;
        };
        tried += 1;
        let Ok(decoded) = special::decode_blob(&derived.special, &blob, d.original as usize) else {
            continue;
        };
        let records = special::parse_records(&decoded)?;
        let mut by_token = BTreeMap::new();
        for (_, name) in &records {
            ensure!(
                by_token
                    .insert(special::token(prefix, name), name.clone())
                    .is_none(),
                "duplicate Special filename token"
            );
        }
        let startup = "startup.tjs".to_owned();
        let mut renamed = Vec::with_capacity(base.entries.len());
        let mut used = 0usize;
        for e in &base.entries {
            if e.path == member {
                renamed.push(Some(startup.clone()));
            } else if let Some(n) = by_token.get(&e.path) {
                used += 1;
                renamed.push(Some(n.clone()));
            } else {
                ensure!(
                    !(e.path.len() == 32 && e.path.bytes().all(|b| b.is_ascii_hexdigit())),
                    "protected XP3 entry {} has no authenticated Special name",
                    e.id
                );
                renamed.push(None);
            }
        }
        ensure!(
            used == records.len(),
            "Special name index: {} of {} records unmapped",
            records.len() - used,
            records.len()
        );
        let real: BTreeSet<String> = renamed.iter().flatten().cloned().collect();
        ensure!(
            real.len() == renamed.iter().flatten().count(),
            "duplicate recovered XP3 path"
        );
        // Unmapped visible names that collide with a recovered path are decoys
        // the native storage never serves (e.g. a placeholder startup.tjs).
        let mut dropped = 0usize;
        let (mut entries, mut plans) = (Vec::new(), Vec::new());
        for ((mut e, plan), name) in std::mem::take(&mut base.entries)
            .into_iter()
            .zip(std::mem::take(&mut base.plans))
            .zip(renamed)
        {
            match name {
                Some(n) => {
                    e.metadata["visible_name"] = serde_json::json!(e.path);
                    e.path = n;
                }
                None if real.contains(&e.path) => {
                    dropped += 1;
                    continue;
                }
                None => {}
            }
            entries.push(e);
            plans.push(plan);
        }
        for (i, e) in entries.iter_mut().enumerate() {
            e.id = i as u32;
        }
        base.entries = entries;
        base.plans = plans;
        let report = serde_json::json!({"records": records.len(), "decoys_dropped": dropped,
            "startup": format!("{archive_name}>{member}")});
        let setup = SetupData {
            text,
            startup_archive: own,
        };
        return Ok((derived, report, setup));
    }
    bail!("XP3 Special index: none of {tried} startup-script constants derives a valid name index")
}

/// Read a member that must be stored in plaintext (its raw bytes match adlr).
fn plain_member(a: &ParsedArchive, member: &str, ctx: &DecodeContext) -> Result<Vec<u8>> {
    let e = a
        .entries
        .iter()
        .find(|e| e.path == member)
        .with_context(|| format!("startup member {member:?} absent"))?;
    let EntryPlan::Segments { segments, .. } = &a.plans[e.id as usize] else {
        bail!("startup member plan");
    };
    let plain = ParsedArchive {
        format: a.format,
        source: a.source.clone(),
        entries: vec![ArchiveEntry { id: 0, ..e.clone() }],
        plans: vec![EntryPlan::Segments {
            segments: segments.clone(),
            encrypted: false,
        }],
    };
    let data = read_all(plain.open(0, ctx)?.as_ref(), 16 * 1024 * 1024)?;
    ensure!(
        Some(u64::from(adler32(&data))) == e.metadata["adler32"].as_u64(),
        "startup member is not plaintext"
    );
    Ok(data)
}

struct Sample {
    hash: u32,
    data: Vec<u8>,
}
fn samples(
    base: &ParsedArchive,
    protected: &BTreeSet<u32>,
    ctx: &DecodeContext,
) -> Result<Vec<Sample>> {
    let mut ids: Vec<&ArchiveEntry> = base
        .entries
        .iter()
        .filter(|e| protected.contains(&e.id) && e.size > 0)
        .collect();
    ids.sort_by_key(|e| (e.size, e.id));
    let mut out = Vec::new();
    let mut total = 0u64;
    for e in ids {
        if out.len() >= SAMPLE_COUNT * 4 || total + e.size > SAMPLE_BYTES {
            break;
        }
        let hash = e.metadata["adler32"].as_u64().context("XP3 adlr absent")? as u32;
        let data = read_all(base.open(e.id, ctx)?.as_ref(), SAMPLE_BYTES)?;
        if adler32(&data) == hash {
            continue;
        }
        total += e.size;
        out.push(Sample { hash, data });
    }
    Ok(out)
}

/// Accept exactly one behaviourally distinct candidate restoring every sample.
fn select(
    candidates: Vec<(engine::CxdecProfile, String)>,
    samples: &[Sample],
) -> Result<Option<(CxdecEngine, String)>> {
    if samples.is_empty() {
        return Ok(None);
    }
    ensure!(
        !candidates.is_empty(),
        "CXDEC: protected entries present but no complete parameter set was recovered from the modules"
    );
    let tests = &samples[..samples.len().min(SAMPLE_COUNT)];
    let passes = |e: &CxdecEngine, set: &[Sample]| {
        set.iter().all(|s| {
            let mut b = s.data.clone();
            e.apply(0, s.hash, &mut b).is_ok() && adler32(&b) == s.hash
        })
    };
    let mut passing: Vec<(CxdecEngine, String)> = Vec::new();
    for (profile, source) in candidates {
        let Ok(e) = CxdecEngine::new(profile) else {
            continue;
        };
        if passes(&e, tests) {
            passing.push((e, source));
        }
    }
    if passing.len() > 1 {
        passing.retain(|(e, _)| passes(e, samples));
    }
    // Distinct parameter sets with identical behaviour are equivalent.
    let fingerprint = |e: &CxdecEngine| {
        let p = e.profile();
        let lanes: Vec<u32> = (0..128u8)
            .flat_map(|l| [0u32, 0x1234_5, 0x7fff_ffff, 0x0abc_def].map(|x| e.evaluate_lane(l, x)))
            .collect();
        (p.mask, p.offset, p.prefix8, lanes)
    };
    let distinct: BTreeSet<_> = passing.iter().map(|(e, _)| fingerprint(e)).collect();
    match distinct.len() {
        0 => bail!(
            "CXDEC: no recovered parameter set restores the archive checksums ({} samples)",
            samples.len()
        ),
        1 => Ok(passing.into_iter().next()),
        n => bail!("CXDEC: {n} distinct parameter sets validate; refusing to guess"),
    }
}

struct CxArchive {
    base: ParsedArchive,
    protected: BTreeSet<u32>,
    engine: Option<Arc<CxdecEngine>>,
    /// Derived, labelled resources (the validated bootstrap profile).
    embedded: BTreeMap<u32, Arc<[u8]>>,
}
impl Archive for CxArchive {
    fn entries(&self) -> &[ArchiveEntry] {
        &self.base.entries
    }
    fn open(&self, id: u32, ctx: &DecodeContext) -> Result<Arc<dyn AssetReader>> {
        if let Some(data) = self.embedded.get(&id) {
            return ctx.memory_shared(data.clone());
        }
        let raw = self.base.open(id, ctx)?;
        if !self.protected.contains(&id) {
            return Ok(raw);
        }
        let e = self
            .base
            .entries
            .get(id as usize)
            .context("CXDEC entry id")?;
        let hash = e.metadata["adler32"].as_u64().context("XP3 adlr absent")? as u32;
        // Verify before publication; already-plain protected entries pass raw.
        let (mut original, mut decoded) = (Adler::default(), Adler::default());
        let mut chunk = vec![0; 65536];
        let mut at = 0u64;
        while at < e.size {
            let n = (e.size - at).min(chunk.len() as u64) as usize;
            crysirror_resource::read_exact_at(raw.as_ref(), at, &mut chunk[..n])?;
            original.update(&chunk[..n]);
            if let Some(engine) = &self.engine {
                engine.apply(at, hash, &mut chunk[..n])?;
                decoded.update(&chunk[..n]);
            }
            at += n as u64;
        }
        if original.finish() == hash {
            return Ok(raw);
        }
        let engine = self
            .engine
            .clone()
            .context("CXDEC protected entry without a validated content profile")?;
        ensure!(
            decoded.finish() == hash,
            "CXDEC decoded entry checksum mismatch: {}",
            e.path
        );
        Ok(Arc::new(FilterReader {
            source: raw,
            engine,
            hash,
        }))
    }
}
struct FilterReader {
    source: Arc<dyn AssetReader>,
    engine: Arc<CxdecEngine>,
    hash: u32,
}
impl AssetReader for FilterReader {
    fn len(&self) -> crysirror_core::Result<u64> {
        self.source.len()
    }
    fn read_at(&self, at: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        let n = self.source.read_at(at, out)?;
        if n > out.len() {
            return Err(crysirror_core::Error::Invalid("CXDEC reader count"));
        }
        self.engine
            .apply(at, self.hash, &mut out[..n])
            .map_err(|_| crysirror_core::Error::Invalid("CXDEC 32-bit entry offset"))?;
        Ok(n)
    }
}

#[derive(Clone, Copy)]
pub(super) struct Adler {
    a: u32,
    b: u32,
}
impl Default for Adler {
    fn default() -> Self {
        Self { a: 1, b: 0 }
    }
}
impl Adler {
    pub(super) fn update(&mut self, data: &[u8]) {
        for chunk in data.chunks(5552) {
            for &v in chunk {
                self.a += u32::from(v);
                self.b += self.a;
            }
            self.a %= 65521;
            self.b %= 65521;
        }
    }
    pub(super) fn finish(self) -> u32 {
        (self.b << 16) | self.a
    }
}
fn adler32(data: &[u8]) -> u32 {
    let mut a = Adler::default();
    a.update(data);
    a.finish()
}

#[cfg(test)]
mod tests {
    use super::engine::{CxdecProfile, GeneratorKind};
    use super::*;
    use crysirror_core::{service::MemoryAssetReader, state::ClientId};

    const BASE: u32 = 0x1000_0000;
    const PROLOG: [u8; 3] = [2, 0, 1];
    const EVEN: [u8; 8] = [0, 7, 5, 6, 3, 1, 4, 2];
    const ODD: [u8; 6] = [4, 3, 2, 1, 5, 0];

    /// Self-authored PE32 exposing a classic generator only through the
    /// structures the recognizers read: callback descriptor, signed control
    /// table, LCG constants and three emitter jump tables. No title data.
    pub(super) fn synthetic_module(mask: u32, offset: u32) -> (Vec<u8>, Vec<u8>) {
        let mut pe = super::recover::tests::synthetic_pe();
        pe.resize(0x4000, 0xcc);
        // .text raw 0x400 (rva 0x1000) .. grow sections to 0x1000 bytes.
        let table = 0x98 + 0xe0;
        for (i, raw) in [(0usize, 0x400u32), (1, 0x1400), (2, 0x2400)] {
            let p = table + i * 40;
            pe[p + 8..p + 12].copy_from_slice(&0x1000u32.to_le_bytes());
            pe[p + 12..p + 16].copy_from_slice(&(0x1000 + 0x1000 * i as u32).to_le_bytes());
            pe[p + 16..p + 20].copy_from_slice(&0x1000u32.to_le_bytes());
            pe[p + 20..p + 24].copy_from_slice(&raw.to_le_bytes());
        }
        pe[0x400..0x1400].fill(0xcc);
        pe[0x1400..0x3400].fill(0);
        let text = |rva: u32| (rva - 0x1000 + 0x400) as usize;
        // .rdata (0x2000) and .data (0x3000) share the file delta 0xc00.
        let rdata = |rva: u32| (rva - 0xc00) as usize;
        // Control table at .rdata start, beginning with the historic signature.
        let mut control = vec![0u8; engine::CONTROL_BLOCK_SIZE];
        let mut x = 0x2468_ace0u32;
        for b in control.iter_mut() {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *b = (x >> 24) as u8;
        }
        control[..25].copy_from_slice(b" Encryption control block");
        pe[rdata(0x2000)..rdata(0x2000) + engine::CONTROL_BLOCK_SIZE].copy_from_slice(&control);
        // Callback descriptor {name=0, mask, offset, builder} referenced by code.
        let cb = rdata(0x3400);
        for (i, v) in [0, mask, offset, BASE + 0x1000].into_iter().enumerate() {
            pe[cb + i * 4..cb + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        let mut code = vec![0x68];
        code.extend((BASE + 0x3400).to_le_bytes());
        code.push(0xb8);
        code.extend(0x41c6_4e6du32.to_le_bytes());
        code.push(0xb8);
        code.extend(0x3039u32.to_le_bytes());
        // jmp [eax*4 + table] for prolog/even/odd selector tables.
        let blocks: [(&[u8], &[&[u8]]); 3] = [
            (
                &PROLOG,
                &[
                    &[0x6a, 0xb8],
                    &[0x6a, 0x8b, 0x6a, 0xc7],
                    &[0x6a, 0xbe, 0x68, 0xff, 3, 0, 0],
                ],
            ),
            (
                &EVEN,
                &[
                    &[0x6a, 0xf7, 0x6a, 0xd0],
                    &[0x6a, 0x48],
                    &[0x6a, 0xf7, 0x6a, 0xd8],
                    &[0x6a, 0x40],
                    &[0x68, 0xff, 3, 0, 0, 0x6a, 0xbe],
                    &[0x68, 0xaa, 0xaa, 0xaa, 0xaa, 0x68, 0x55, 0x55, 0x55, 0x55],
                    &[0x6a, 0x35],
                    &[0x6a, 0x05],
                ],
            ),
            (
                &ODD,
                &[
                    &[0x6a, 0xd3, 0x6a, 0xe8],
                    &[0x6a, 0xd3, 0x6a, 0xe0],
                    &[0x6a, 0x01, 0x6a, 0xd8],
                    &[0x6a, 0xf7, 0x6a, 0xd8, 0x6a, 0x01],
                    &[0x6a, 0x0f, 0x6a, 0xaf, 0x6a, 0xc3],
                    &[0x6a, 0x29, 0x6a, 0xd8],
                ],
            ),
        ];
        for (k, (order, kinds)) in blocks.into_iter().enumerate() {
            let tva = BASE + 0x3500 + k as u32 * 0x40;
            code.extend([0xff, 0x24, 0x85]);
            code.extend(tva.to_le_bytes());
            let group = 0x1200 + k as u32 * 0x300;
            for (slot, &sem) in order.iter().enumerate() {
                let rva = group + u32::from(sem) * 0x10;
                let o = text(rva);
                pe[o..o + kinds[sem as usize].len()].copy_from_slice(kinds[sem as usize]);
                let t = rdata(tva - BASE) + slot * 4;
                pe[t..t + 4].copy_from_slice(&(BASE + rva).to_le_bytes());
            }
        }
        pe[text(0x1000)..text(0x1000) + code.len()].copy_from_slice(&code);
        (pe, control)
    }

    fn xp3_archive(files: &[(&str, Vec<u8>, u32, bool)]) -> Vec<u8> {
        let mut out = b"XP3\r\n \n\x1a\x8bg\x01".to_vec();
        out.extend(0u64.to_le_bytes());
        let mut index = Vec::new();
        for (name, stored, adlr, encrypted) in files {
            let offset = out.len() as u64;
            out.extend(stored);
            let mut segm = 0u32.to_le_bytes().to_vec();
            segm.extend(offset.to_le_bytes());
            segm.extend((stored.len() as u64).to_le_bytes());
            segm.extend((stored.len() as u64).to_le_bytes());
            let units: Vec<u16> = name.encode_utf16().collect();
            let mut info = (if *encrypted { 0x8000_0000u32 } else { 0 })
                .to_le_bytes()
                .to_vec();
            info.extend((stored.len() as u64).to_le_bytes());
            info.extend((stored.len() as u64).to_le_bytes());
            info.extend((units.len() as u16).to_le_bytes());
            info.extend(units.iter().flat_map(|u| u.to_le_bytes()));
            let mut file = chunk(b"adlr", &adlr.to_le_bytes());
            file.extend(chunk(b"segm", &segm));
            file.extend(chunk(b"info", &info));
            index.extend(chunk(b"File", &file));
        }
        let at = out.len() as u64;
        out[11..19].copy_from_slice(&at.to_le_bytes());
        out.push(0);
        out.extend((index.len() as u64).to_le_bytes());
        out.extend(index);
        out
    }
    fn chunk(tag: &[u8], body: &[u8]) -> Vec<u8> {
        let mut c = tag.to_vec();
        c.extend((body.len() as u64).to_le_bytes());
        c.extend(body);
        c
    }
    fn context(module: Vec<u8>) -> DecodeContext {
        let mut ctx = DecodeContext::new(ClientId(3));
        ctx.source_name = Some("data.xp3".into());
        let module: Arc<[u8]> = module.into();
        ctx.resolver = Some(Arc::new(
            move |path: &str| -> Result<Arc<dyn AssetReader>> {
                ensure!(path == "plugin/game.tpm", "unexpected companion {path}");
                Ok(Arc::new(MemoryAssetReader(module.clone())))
            },
        ));
        ctx.parameters.insert(
            "kirikiri/xp3".into(),
            serde_json::json!({"cxdec_modules": ["plugin/game.tpm"]}),
        );
        ctx
    }
    fn plaintexts() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            (
                "startup.tjs",
                b"Scripts.execStorage(\"system/Initialize.tjs\");".to_vec(),
            ),
            ("image/a.bin", (0..3000u32).map(|i| (i * 7) as u8).collect()),
            (
                "image/b.bin",
                (0..900u32).map(|i| (i ^ 0x5a) as u8).collect(),
            ),
            ("plain.txt", b"never filtered".to_vec()),
        ]
    }

    #[test]
    fn synthetic_module_parameters_are_recovered_validated_and_applied() {
        let (mask, offset) = (0x1a3, 0x0b6);
        let (module, control) = synthetic_module(mask, offset);
        let facts = recover::analyze("plugin/game.tpm", &module);
        let generic = recover::generic_candidates(&facts);
        assert!(generic.iter().any(|(p, _)| p.mask == mask
            && p.prolog_order == PROLOG
            && p.even_branch_order == EVEN
            && p.odd_branch_order == ODD
            && p.control_block == control));
        let engine = CxdecEngine::new(CxdecProfile {
            mask,
            offset,
            prolog_order: PROLOG,
            even_branch_order: EVEN,
            odd_branch_order: ODD,
            control_block: control,
            generator: GeneratorKind::Classic,
            prefix8: None,
        })
        .unwrap();
        let files: Vec<_> = plaintexts()
            .into_iter()
            .map(|(n, p)| {
                let adlr = adler32(&p);
                let mut c = p.clone();
                let encrypted = n != "plain.txt";
                if encrypted {
                    engine.apply(0, adlr, &mut c).unwrap();
                }
                (n, c, adlr, encrypted)
            })
            .collect();
        let data = xp3_archive(&files);
        let ctx = context(module.clone());
        let archive = Xp3
            .open(Arc::new(MemoryAssetReader(data.clone().into())), &ctx)
            .unwrap();
        for (e, (_, plain)) in archive.entries().iter().zip(plaintexts()) {
            let got = read_all(archive.open(e.id, &ctx).unwrap().as_ref(), 1 << 20).unwrap();
            assert_eq!(got, plain, "{}", e.path);
            // Ranged reads through the filter match the whole-entry transform.
            if e.size > 600 {
                let r = archive.open(e.id, &ctx).unwrap();
                let mut part = vec![0; 300];
                crysirror_resource::read_exact_at(r.as_ref(), 250, &mut part).unwrap();
                assert_eq!(part, plain[250..550]);
            }
        }
        // Without a profile the protected entries stay unreadable.
        let mut bare = ctx.clone();
        bare.parameters.clear();
        let raw = Xp3
            .open(Arc::new(MemoryAssetReader(data.clone().into())), &bare)
            .unwrap();
        assert!(raw.open(1, &bare).is_err());
        // A corrupted ciphertext byte is detected before publication.
        let mut bad = data.clone();
        let at = 11 + 8 + files[0].1.len() + 10;
        bad[at] ^= 1;
        assert!(Xp3
            .open(Arc::new(MemoryAssetReader(bad.into())), &ctx)
            .and_then(|a| a.open(1, &ctx))
            .is_err());
        // Wrong parameters: the archive refuses to open instead of guessing.
        let (other, _) = synthetic_module(0x1a3, 0x0b7);
        assert!(Xp3
            .open(Arc::new(MemoryAssetReader(data.into())), &context(other))
            .is_err());
    }

    #[test]
    fn modules_without_generator_evidence_or_profiles_fail_visibly() {
        let files = vec![("a", vec![1u8, 2, 3], 7u32, true)];
        let data = xp3_archive(&files);
        let mut module = super::recover::tests::synthetic_pe();
        module.resize(0x1000, 0);
        let err = Xp3
            .open(Arc::new(MemoryAssetReader(data.into())), &context(module))
            .err()
            .unwrap();
        assert!(format!("{err:#}").contains("CXDEC"), "{err:#}");
        // An indirect Special descriptor without an explicit profile is refused.
        let mut x = xp3_archive(&[("a", vec![1], 7, false)]);
        let mut special = 23u64.to_le_bytes().to_vec();
        special.extend(1u32.to_le_bytes());
        special.extend(1u32.to_le_bytes());
        let idx = u64::from_le_bytes(x[11..19].try_into().unwrap()) as usize;
        let mut table = chunk(b"zzz:", &special);
        table.extend(&x[idx + 9..]);
        x.truncate(idx + 1);
        x.extend((table.len() as u64).to_le_bytes());
        x.extend(table);
        let err = Xp3
            .open(
                Arc::new(MemoryAssetReader(x.into())),
                &DecodeContext::new(ClientId(3)),
            )
            .err()
            .unwrap();
        assert!(format!("{err:#}").contains("Special"), "{err:#}");
    }

    #[test]
    fn special_names_come_from_startup_constants_and_cover_every_record() {
        use std::io::Write;
        let text = "synthetic-archive-data-0123456789";
        let derived = special::derive(text).unwrap();
        // Compiled-script string pool carrying the setupArchiveData argument.
        let mut pool = Vec::new();
        for _ in 0..5 {
            pool.extend(0u32.to_le_bytes());
        }
        let strings = ["Storages", "setupArchiveData", text];
        pool.extend((strings.len() as u32).to_le_bytes());
        for s in strings {
            pool.extend((s.len() as u32).to_le_bytes());
            pool.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
            pool.resize((pool.len() + 3) & !3, 0);
        }
        let mut script = b"TJS2100\0".to_vec();
        script.extend(0u32.to_le_bytes());
        script.extend(b"DATA");
        script.extend((pool.len() as u32).to_le_bytes());
        script.extend(pool);
        let names = ["system/Initialize.tjs", "image/a.png"];
        let mut records = Vec::new();
        for n in names {
            records.extend(special::tests::record(n));
        }
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&records).unwrap();
        let mut blob = z.finish().unwrap();
        let n = blob.len().min(0x100);
        derived.special.apply(&mut blob[..n]);
        let mut files = vec![
            ("$".to_owned(), script.clone(), adler32(&script), false),
            (
                "startup.tjs".to_owned(),
                b"decoy".to_vec(),
                adler32(b"decoy"),
                false,
            ),
        ];
        for n in names {
            files.push((
                special::token("tst:", n),
                n.as_bytes().to_vec(),
                adler32(n.as_bytes()),
                false,
            ));
        }
        let refs: Vec<_> = files
            .iter()
            .map(|(n, d, a, e)| (n.as_str(), d.clone(), *a, *e))
            .collect();
        let mut x = xp3_archive(&refs);
        let idx = u64::from_le_bytes(x[11..19].try_into().unwrap()) as usize;
        let tail = x[idx + 9..].to_vec();
        x.truncate(idx);
        x.extend(&blob);
        let new_idx = x.len() as u64;
        x[11..19].copy_from_slice(&new_idx.to_le_bytes());
        let mut special = (idx as u64).to_le_bytes().to_vec();
        special.extend((records.len() as u32).to_le_bytes());
        special.extend((blob.len() as u32).to_le_bytes());
        let mut table = chunk(b"tst:", &special);
        table.extend(tail);
        x.push(0);
        x.extend((table.len() as u64).to_le_bytes());
        x.extend(table);
        let index = parse_index(
            Arc::new(MemoryAssetReader(x.into())),
            &DecodeContext::new(ClientId(3)),
        )
        .unwrap();
        let mut base = index.archive;
        let facts = vec![ModuleFacts {
            setup_generator: true,
            startup_redirect: Some(("data.xp3".into(), "$".into())),
            ..Default::default()
        }];
        let mut ctx = DecodeContext::new(ClientId(3));
        ctx.source_name = Some("game/data.xp3".into());
        let (d, report, setup) =
            apply_special_names(&mut base, &index.specials[0], &facts, &ctx).unwrap();
        assert_eq!(d.special, derived.special);
        // The accepted setupArchiveData argument is retained for the native
        // facade, and only the startup archive publishes the profile.
        assert_eq!(setup.text, text);
        assert!(setup.startup_archive);
        let profile: serde_json::Value =
            serde_json::from_slice(&bootstrap_profile(&setup, &report, &None).unwrap()).unwrap();
        assert_eq!(profile["family"], "cxdec-setup-archive-data");
        assert_eq!(profile["setup_archive_data"], text);
        assert_eq!(profile["setup_archive_data_units"], text.len());
        assert_eq!(profile["startup"], "data.xp3>$");
        assert_eq!(profile["records"], 2);
        let paths: Vec<_> = base.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            ["startup.tjs", "system/Initialize.tjs", "image/a.png"]
        );
        let startup = read_all(base.open(0, &ctx).unwrap().as_ref(), 1 << 20).unwrap();
        assert_eq!(startup, script);
    }
}
