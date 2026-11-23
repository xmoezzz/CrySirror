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
use std::sync::Mutex;

const VERIFIED_STREAM_SLOTS: usize = 128;
type VerifiedStream = (u32, Arc<dyn AssetReader>);

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
    let mut lookups = BTreeMap::<_, BTreeMap<_, _>>::new();
    let mut protected_ids = std::collections::BTreeSet::new();
    for entry in &mut base.entries {
        let fake = fake_id(&entry.path);
        let plan = &mut base.plans[entry.id as usize];
        let protected = matches!(plan, EntryPlan::Segments { facts, .. } if facts.flags & 0x8000_0000 != 0);
        if let EntryPlan::Segments { encrypted, .. } = plan {
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
                lookups.entry(r.path_hash).or_default()
                    .insert(r.name_hash, entry.id)
                    .is_none(),
                "duplicate HXV4 hashed path"
            );
            if let EntryPlan::Segments { facts, .. } = &mut base.plans[entry.id as usize] {
                facts.hxv4_id = Some(r.id);
            }
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
                !base
                    .entries
                    .iter()
                    .any(|e| e.path == root || e.path.starts_with(&format!("{root}/"))),
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
    if let Some(target) = options
        .get("hxv4_bootstrap_profile")
        .and_then(|v| v.as_str())
    {
        derived.push((
            target,
            bootstrap_profile(&analysis, &recovered, exe, hashed_root.as_deref())?,
            "validated HXV4 bootstrap profile",
        ));
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
        base.entries.push(ArchiveEntry {
            id,
            path: target.into(),
            size: data.len() as u64,
            stored_size: 0,
            metadata: serde_json::json!({"format":"kirikiri/xp3","origin":origin,"derived":true}),
        });
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
        verified_streams: std::array::from_fn(|_| Mutex::new(None)),
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
    let token: String = token
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
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
    lookups: BTreeMap<[u8; 8], BTreeMap<[u8; 32], u32>>,
    manager: native::Hxv4NativeFilterManager,
    // These cache only a small number of zero-copy stream handles. A slot
    // belongs to this authenticated catalog and its original source handle.
    verified_streams: [Mutex<Option<VerifiedStream>>; VERIFIED_STREAM_SLOTS],
}
impl Archive for HxArchive {
    fn entries(&self) -> &[ArchiveEntry] {
        &self.base.entries
    }
    fn entry_metadata(&self, id: u32) -> serde_json::Value {
        self.base.entry_metadata(id)
    }
    fn resolve(&self, path: &str) -> Option<u32> {
        let path = path.replace('\\', "/");
        let path = match &self.hashed_root {
            Some(root) => path
                .strip_prefix(root.as_str())?
                .strip_prefix('/')?
                .to_owned(),
            None => path,
        };
        // Directory spelling keeps its trailing delimiter; the root is "".
        let cut = path.rfind('/').map_or(0, |i| i + 1);
        let (dir, name) = path.split_at(cut);
        let folded_dir = dir.to_lowercase();
        let mut exact_name = None;
        for spelling in std::iter::once(dir).chain((folded_dir != dir).then_some(folded_dir.as_str())) {
            let Some(files) = self.lookups.get(&hxv4_path_hash(spelling)) else { continue };
            if let Some(id) = files.get(exact_name.get_or_insert_with(|| hxv4_filename_hash(name))) {
                return Some(*id);
            }
            let folded = name.to_lowercase();
            if folded != name {
                if let Some(id) = files.get(&hxv4_filename_hash(&folded)) { return Some(*id); }
            }
        }
        None
    }
    fn open(&self, id: u32, ctx: &DecodeContext) -> Result<Arc<dyn AssetReader>> {
        ctx.check_cancelled()?;
        if let Some(data) = self.embedded.get(&id) {
            return ctx.memory_shared(data.clone());
        }
        if !self.protected_ids.contains(&id) {
            return self.base.open(id, ctx);
        }
        let e = self
            .base
            .entries
            .get(id as usize)
            .context("HXV4 entry id")?;
        ensure!(e.size <= ctx.limits.max_entry_size, "archive entry decoded byte limit");
        if let EntryPlan::Segments { segments, .. } = &self.base.plans[id as usize] {
            if let [segment] = segments.as_slice() {
                if self.base.source.is_immutable()
                    && !segment.compressed
                    && segment.stored == e.size
                    && segment.size == e.size
                    && ctx.pipeline(self.base.format, e)?.is_empty()
                {
                    return self.open_verified_stream(id, segment.offset, segment.size, ctx);
                }
            }
        }
        self.verify(id, e.size, self.base.open(id, ctx)?, ctx)
    }
}
impl HxArchive {
    fn open_verified_stream(&self, id: u32, offset: u64, size: u64, ctx: &DecodeContext) -> Result<Arc<dyn AssetReader>> {
        let mut slot = self.verified_streams[id as usize % VERIFIED_STREAM_SLOTS]
            .lock()
            .map_err(|_| anyhow::anyhow!("HXV4 verified stream lock poisoned"))?;
        ctx.check_cancelled()?;
        if let Some((_, stream)) = slot.as_ref().filter(|(cached_id, _)| *cached_id == id) {
            return Ok(stream.clone());
        }
        let raw = subrange(self.base.source.clone(), offset, size)?;
        let verified = self.verify(id, size, raw, ctx)?;
        ctx.check_cancelled()?;
        *slot = Some((id, verified.clone()));
        Ok(verified)
    }

    fn verify(
        &self,
        id: u32,
        size: u64,
        raw: Arc<dyn AssetReader>,
        ctx: &DecodeContext,
    ) -> Result<Arc<dyn AssetReader>> {
        let expected = self.base.xp3(id).context("HXV4 checksum absent")?.adler32;
        let state = self
            .filters
            .get(&id)
            .map(|&(key, flag)| self.manager.state_for_entry(key, flag));
        // Validate incrementally before publication. Raw bootstrap entries may
        // carry the protected bit while already containing the exact plaintext.
        let (mut original, mut decoded) = (Adler::new(), Adler::new());
        let mut chunk = vec![0; 65536];
        let mut at = 0u64;
        while at < size {
            ctx.check_cancelled()?;
            let n = (size - at).min(chunk.len() as u64) as usize;
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

#[cfg(test)]
mod verified_stream_tests {
    use super::*;
    use std::sync::{atomic::{AtomicUsize, Ordering}, Barrier};

    struct Source {
        bytes: Mutex<Vec<u8>>,
        reads: AtomicUsize,
        immutable: bool,
    }

    impl AssetReader for Source {
        fn len(&self) -> crysirror_core::Result<u64> {
            Ok(self.bytes.lock().unwrap().len() as u64)
        }

        fn read_at(&self, offset: u64, output: &mut [u8]) -> crysirror_core::Result<usize> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            let bytes = self.bytes.lock().unwrap();
            let tail = bytes.get(offset as usize..).unwrap_or_default();
            let len = output.len().min(tail.len());
            output[..len].copy_from_slice(&tail[..len]);
            Ok(len)
        }

        fn is_immutable(&self) -> bool {
            self.immutable
        }
    }

    fn fixture(immutable: bool) -> (HxArchive, Arc<Source>, Vec<u8>) {
        use crate::common::{Segment, Xp3Facts};

        let params = [0, 1, 2, 3, 4, 5, 6, 7, 0, 1, 2, 3, 4, 5, 0, 1, 2, 0, 52, 18, 120, 86];
        let table: Vec<_> = (0..1024u32).map(|x| x.rotate_left(7)).collect();
        let manager = native::Hxv4NativeFilterManager::from_params_and_table(&params, &table).unwrap();
        let plain = vec![43; 150_000];
        let mut checksum = Adler::new();
        checksum.update(&plain);
        let size = plain.len() as u64;
        let key = 0x1122_3344_5566_7788;
        let mut stored = plain.clone();
        manager.apply_entry(key, 1, 0, &mut stored);
        let source = Arc::new(Source { bytes: Mutex::new(stored), reads: AtomicUsize::new(0), immutable });
        let archive = HxArchive {
            embedded: BTreeMap::new(),
            hashed_root: None,
            protected_ids: [0].into(),
            base: ParsedArchive {
                format: "kirikiri/xp3",
                source: source.clone(),
                entries: vec![ArchiveEntry { id: 0, path: "fixture.bin".into(), size, stored_size: size, metadata: serde_json::Value::Null }],
                plans: vec![EntryPlan::Segments {
                    segments: vec![Segment { offset: 0, stored: size, size, compressed: false }],
                    encrypted: false,
                    facts: Xp3Facts { adler32: checksum.finish(), ..Default::default() },
                }],
            },
            filters: [(0, (key, 1))].into(),
            lookups: BTreeMap::new(),
            manager,
            verified_streams: std::array::from_fn(|_| Mutex::new(None)),
        };
        (archive, source, plain)
    }

    #[test]
    fn concurrent_opens_share_one_verified_stream() {
        let (archive, source, plain) = fixture(true);
        let ctx = DecodeContext::new(crysirror_core::state::ClientId(1));
        let ready = Barrier::new(2);
        let one = std::thread::scope(|threads| {
            let other = threads.spawn(|| {
                ready.wait();
                archive.open(0, &ctx).unwrap()
            });
            ready.wait();
            let one = archive.open(0, &ctx).unwrap();
            assert!(Arc::ptr_eq(&one, &other.join().unwrap()));
            one
        });
        assert_eq!(source.reads.load(Ordering::Relaxed), 3);
        let mut part = [0; 32];
        assert_eq!(one.read_at(65_531, &mut part).unwrap(), part.len());
        assert_eq!(part, plain[65_531..65_563]);
        let mut limited = ctx.clone();
        limited.limits.max_entry_size = 32;
        assert!(archive.open(0, &limited).is_err());
        let cancelled = crysirror_tasks::Cancellation::default();
        cancelled.cancel();
        assert!(archive.open(0, &ctx.with_cancellation(cancelled)).is_err());
    }

    #[test]
    fn mutable_sources_are_revalidated_and_corruption_is_rejected() {
        let (archive, source, _) = fixture(false);
        let ctx = DecodeContext::new(crysirror_core::state::ClientId(1));
        assert!(archive.open(0, &ctx).is_ok());
        source.bytes.lock().unwrap()[30] ^= 128;
        let failure = archive.open(0, &ctx).err().expect("corruption must fail");
        assert!(failure.to_string().contains("checksum mismatch"));
        source.bytes.lock().unwrap()[30] ^= 128;
        assert!(archive.open(0, &ctx).is_ok());
        assert_eq!(source.reads.load(Ordering::Relaxed), 9);
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
