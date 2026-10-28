// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Shared authenticated HXV4 resource profile. No executable is run.
mod bootstrap;
mod index;
mod native;
use super::*;
use crate::common::{EntryPlan, ParsedArchive};
use crysirror_resource::read_all;
use index::{hxv4_filename_hash, hxv4_path_hash};

pub(super) struct Descriptor {
    pub offset: u64,
    pub size: u64,
    pub flags: u16,
}
pub(super) fn descriptor(body: &[u8], size: u64) -> Result<Descriptor> {
    ensure!(body.len() == 14, "HXV4 descriptor size");
    let d = Descriptor {
        offset: u64::from_le_bytes(body[..8].try_into()?),
        size: u64::from(u32::from_le_bytes(body[8..12].try_into()?)),
        flags: u16::from_le_bytes(body[12..].try_into()?),
    };
    ensure!(d.flags & !1 == 0, "unsupported HXV4 descriptor flags");
    range(size, d.offset, d.size)?;
    ensure!(
        d.size >= 21 && d.size <= 16 * 1024 * 1024,
        "HXV4 Special byte limit"
    );
    Ok(d)
}
pub(super) fn open(
    mut base: ParsedArchive,
    d: Descriptor,
    ctx: &DecodeContext,
) -> Result<Box<dyn Archive>> {
    let options = ctx
        .parameters
        .get("kirikiri/xp3")
        .context("HXV4 needs explicit kirikiri/xp3 profile with companion executable")?;
    let exe = options
        .get("hxv4_executable")
        .and_then(|v| v.as_str())
        .context("HXV4 needs hxv4_executable capability-relative companion name")?;
    let source = ctx.open_sibling(exe)?;
    let exe_bytes = read_all(source.as_ref(), 32 * 1024 * 1024)?;
    let analysis = bootstrap::analyze_bytes(&exe_bytes).map_err(anyhow::Error::msg)?;
    ensure!(
        d.size <= ctx.limits.max_index_size,
        "HXV4 stored index limit"
    );
    let special = bytes_at(&base.source, &d)?;
    let recovered = bootstrap::recover_hxv4_keys_from_analysis(
        &analysis,
        &special,
        d.flags,
        ctx.limits.max_index_size.min(16 * 1024 * 1024) as usize,
    )
    .map_err(anyhow::Error::msg)?;
    ensure!(
        recovered.index.entries.len() <= ctx.limits.max_entries,
        "HXV4 Special entry count"
    );
    // The ordinary bootstrap can be plaintext even with the encrypted bit set.
    // It must not be attached to a Special record merely by basename.
    if let Some(i) = base
        .entries
        .iter()
        .position(|e| fake_id(&e.path) == Some(0))
    {
        const WARNING: &[u8] = b"Warning: Extracting this archive may infringe on author's rights.";
        if let EntryPlan::Segments { segments, .. } = &base.plans[i] {
            if segments.len() == 1
                && !segments[0].compressed
                && segments[0].stored >= WARNING.len() as u64
                && bytes(
                    base.source.as_ref(),
                    segments[0].offset,
                    WARNING.len() as u64,
                    WARNING.len() as u64,
                )? == WARNING
            {
                base.entries.remove(i);
                base.plans.remove(i);
                for (i, e) in base.entries.iter_mut().enumerate() {
                    e.id = i as u32;
                }
            }
        }
    }
    let mut records = BTreeMap::new();
    for r in &recovered.index.entries {
        if r.archive_slot == 0 {
            ensure!(records.insert(r.id, r).is_none(), "duplicate HXV4 id");
        }
    }
    let mut filters = BTreeMap::new();
    let mut lookups = BTreeMap::new();
    let mut protected_ids = std::collections::BTreeSet::new();
    for entry in &mut base.entries {
        let fake = fake_id(&entry.path);
        let protected = entry.metadata["flags"].as_u64().unwrap_or(0) & 0x8000_0000 != 0;
        if let EntryPlan::Segments { encrypted, .. } = &mut base.plans[entry.id as usize] {
            *encrypted = false;
        }
        if protected {
            protected_ids.insert(entry.id);
        }
        let record = fake.and_then(|id| records.get(&id).copied());
        if let Some(r) = record {
            if protected {
                filters.insert(entry.id, (r.entry_key, r.filter_flag));
            }
            ensure!(
                lookups
                    .insert((r.path_hash, r.name_hash), entry.id)
                    .is_none(),
                "duplicate HXV4 hashed path"
            );
            entry.metadata["hxv4"] =
                serde_json::json!({"authenticated":true,"name":"hashed","id":r.id});
        } else if protected && fake.is_some() {
            anyhow::bail!(
                "HXV4 protected entry {} ({:?}, lookup {:?}) missing authenticated Special record",
                entry.id,
                entry.path,
                fake
            )
        }
    }
    // Optional reserved directory for hashed (compound-media) names. Plain XP3
    // names such as the ordinary sentinel startup then never shadow them.
    let hashed_root = match options.get("hxv4_hashed_root").and_then(|v| v.as_str()) {
        Some(root) => {
            let path = crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, root)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            ensure!(
                path.relative() == root && !root.contains('/') && root.len() <= 64,
                "HXV4 hashed root must be one canonical path segment"
            );
            ensure!(
                !base.entries.iter().any(|e| e.path == root || e.path.starts_with(&format!("{root}/"))),
                "HXV4 hashed root collides with an archive entry"
            );
            Some(root.to_owned())
        }
        None => None,
    };
    let mut derived = Vec::new();
    if let Some(target) = options.get("hxv4_startup_target").and_then(|v| v.as_str()) {
        let data: Arc<[u8]> = Arc::from(analysis.startup_tjs.clone());
        derived.push((target, data, "companion PE RCDATA/STARTUP.TJS"));
    }
    if let Some(target) = options.get("hxv4_bootstrap_profile").and_then(|v| v.as_str()) {
        derived.push((target, bootstrap_profile(&analysis, &recovered, exe, hashed_root.as_deref())?, "validated HXV4 bootstrap profile"));
    }
    let mut embedded = BTreeMap::new();
    for (target, data, origin) in derived {
        let path = crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, target)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        ensure!(
            path.relative() == target && target.len() <= 512,
            "HXV4 bootstrap target must be canonical"
        );
        ensure!(
            base.entries.len() < ctx.limits.max_entries
                && !base.entries.iter().any(|e| e.path == target),
            "HXV4 bootstrap target collision or count limit"
        );
        ensure!(
            data.len() as u64 <= ctx.limits.max_entry_size,
            "HXV4 bootstrap byte limit"
        );
        let id = base.entries.len() as u32;
        base.entries.push(ArchiveEntry{id,path:target.into(),size:data.len()as u64,stored_size:0,metadata:serde_json::json!({"format":"kirikiri/xp3","origin":origin,"derived":true})});
        embedded.insert(id, data);
    }
    Ok(Box::new(HxArchive {
        base,
        filters,
        lookups,
        protected_ids,
        embedded,
        hashed_root,
        manager: recovered.native_filter,
    }))
}
/// Engine-owned description of the validated bootstrap, consumed by native
/// facades (`Plugins.linkZ`) without re-running PE analysis. It carries no
/// archive key: `context_token` is a one-way digest identifying this recovery.
fn bootstrap_profile(
    analysis: &bootstrap::KrkrExeAnalysis,
    recovered: &bootstrap::Hxv4ExeKeyRecovery,
    exe: &str,
    hashed_root: Option<&str>,
) -> Result<Arc<[u8]>> {
    use sha2::{Digest, Sha256};
    let public_key = bootstrap::bootstrap_public_key(analysis).map_err(anyhow::Error::msg)?;
    let mut token = Sha256::new();
    token.update(b"CrySirror HXV4 bootstrap context v1\0");
    token.update(recovered.key);
    token.update(analysis.bootstrap_bres_path.as_bytes());
    token.update(recovered.bootstrap_prefix.as_bytes());
    let token: String = token.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let profile = serde_json::json!({
        "family": "hxv4-bootstrap",
        "executable": exe,
        "bootstrap_storage": format!("bres://./{}/bootstrap", analysis.bootstrap_bres_path),
        "bootstrap_prefix": recovered.bootstrap_prefix,
        "archive_unique_key": recovered.archive_unique_key,
        "media": "xp3hnp",
        "hashed_root": hashed_root,
        "context_token": token,
        "signature_public_key": String::from_utf8(public_key)?,
    });
    Ok(Arc::from(serde_json::to_vec_pretty(&profile)?))
}
fn bytes_at(source: &Arc<dyn AssetReader>, d: &Descriptor) -> Result<Vec<u8>> {
    bytes(source.as_ref(), d.offset, d.size, 16 * 1024 * 1024)
}
struct HxArchive {
    embedded: BTreeMap<u32, Arc<[u8]>>,
    hashed_root: Option<String>,
    protected_ids: std::collections::BTreeSet<u32>,
    base: ParsedArchive,
    filters: BTreeMap<u32, (u64, u16)>,
    lookups: BTreeMap<([u8; 8], [u8; 32]), u32>,
    manager: native::Hxv4NativeFilterManager,
}
impl Archive for HxArchive {
    fn entries(&self) -> &[ArchiveEntry] {
        &self.base.entries
    }
    fn resolve(&self, path: &str) -> Option<u32> {
        let path = path.replace('\\', "/");
        let path = match &self.hashed_root {
            Some(root) => path.strip_prefix(root.as_str())?.strip_prefix('/')?.to_owned(),
            None => path,
        };
        // Directory spelling keeps its trailing delimiter; the root is "".
        let cut = path.rfind('/').map_or(0, |i| i + 1);
        let (dir, name) = path.split_at(cut);
        for dir in [dir.to_owned(), dir.to_lowercase()] {
            for name in [name.to_owned(), name.to_lowercase()] {
                if let Some(id) = self
                    .lookups
                    .get(&(hxv4_path_hash(&dir), hxv4_filename_hash(&name)))
                {
                    return Some(*id);
                }
            }
        }
        None
    }
    fn open(&self, id: u32, ctx: &DecodeContext) -> Result<Arc<dyn AssetReader>> {
        if let Some(data) = self.embedded.get(&id) {
            return ctx.memory_shared(data.clone());
        }
        let raw = self.base.open(id, ctx)?;
        if !self.protected_ids.contains(&id) {
            return Ok(raw);
        }
        let e = self
            .base
            .entries
            .get(id as usize)
            .context("HXV4 entry id")?;
        let expected = e.metadata["adler32"]
            .as_u64()
            .context("HXV4 checksum absent")? as u32;
        let state = self
            .filters
            .get(&id)
            .map(|&(key, flag)| self.manager.state_for_entry(key, flag));
        // Validate incrementally before publication. Raw bootstrap entries may
        // carry the protected bit while already containing the exact plaintext.
        let (mut original, mut decoded) = (Adler::new(), Adler::new());
        let mut chunk = vec![0; 65536];
        let mut at = 0u64;
        while at < e.size {
            let n = (e.size - at).min(chunk.len() as u64) as usize;
            crysirror_resource::read_exact_at(raw.as_ref(), at, &mut chunk[..n])?;
            original.update(&chunk[..n]);
            if let Some(state) = &state {
                state.apply(at, &mut chunk[..n]);
                decoded.update(&chunk[..n]);
            }
            at += n as u64;
        }
        if original.finish() == expected {
            return Ok(raw);
        }
        let state = state.context("HXV4 encrypted entry has no authenticated filter state")?;
        ensure!(
            decoded.finish() == expected,
            "HXV4 decoded entry checksum mismatch"
        );
        Ok(Arc::new(FilterReader { source: raw, state }))
    }
}
struct FilterReader {
    source: Arc<dyn AssetReader>,
    state: native::Hxv4NativeFilterState,
}
impl AssetReader for FilterReader {
    fn len(&self) -> crysirror_core::Result<u64> {
        self.source.len()
    }
    fn read_at(&self, at: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        let n = self.source.read_at(at, out)?;
        if n > out.len() {
            return Err(crysirror_core::Error::Invalid("HXV4 reader count"));
        }
        self.state.apply(at, &mut out[..n]);
        Ok(n)
    }
}
struct Adler {
    a: u32,
    b: u32,
}
impl Adler {
    fn new() -> Self {
        Self { a: 1, b: 0 }
    }
    fn update(&mut self, data: &[u8]) {
        for chunk in data.chunks(5552) {
            for &v in chunk {
                self.a += u32::from(v);
                self.b += self.a;
            }
            self.a %= 65521;
            self.b %= 65521;
        }
    }
    fn finish(&self) -> u32 {
        (self.b << 16) | self.a
    }
}

fn fake_id(name: &str) -> Option<u64> {
    let words: Vec<u16> = name.encode_utf16().collect();
    if words.is_empty() || words.len() > 3 {
        return None;
    }
    let mut value = 0u64;
    for (i, w) in words.iter().enumerate() {
        if !(0x5000..=0x8fff).contains(w) {
            return None;
        }
        let part = u64::from(w - 0x5000);
        if i > 0 && i == words.len() - 1 && part == 0 {
            return None;
        }
        value |= part << (i * 14);
    }
    (value <= u64::from(u32::MAX)).then_some(value)
}
