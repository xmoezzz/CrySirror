// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Publish the native inline preferred-name namespace while retaining info-token
//! aliases and the IDs of shadowed plain-index records. Entry::preferred_name
//! follows xp3-brute c82cb14302138125640478556723705aa720007b (MIT).
use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn publish(inner: Box<dyn Archive>) -> Result<Box<dyn Archive>> {
    if !inner
        .entries()
        .iter()
        .any(|e| e.metadata["xp3_inline_name"].is_string())
    {
        return Ok(inner);
    }
    let mut names: BTreeMap<String, Vec<&ArchiveEntry>> = BTreeMap::new();
    let mut aliases = BTreeMap::new();
    for entry in inner.entries() {
        let path = crysirror_core::path::VfsPath::new(
            crysirror_core::path::Mount::Asset,
            &entry.path.replace('\\', "/"),
        )
        .map_err(|e| anyhow::anyhow!("XP3 entry path: {e}"))?;
        names
            .entry(path.relative().to_owned())
            .or_default()
            .push(entry);
        if entry.metadata["xp3_inline_name"].is_string() {
            let native = entry.metadata["xp3_info_name"]
                .as_str()
                .context("XP3 inline name without native info name")?
                .replace('\\', "/");
            let native =
                crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, &native)
                    .map_err(|e| anyhow::anyhow!("XP3 native info alias: {e}"))?;
            ensure!(
                aliases
                    .insert(native.relative().to_owned(), entry.id)
                    .is_none(),
                "ambiguous XP3 native info alias"
            );
        }
    }
    let mut hidden = BTreeSet::new();
    let mut shadows = BTreeMap::new();
    for (name, entries) in &names {
        if entries.len() < 2 {
            continue;
        }
        let preferred: Vec<_> = entries
            .iter()
            .filter(|e| e.metadata["xp3_inline_name"].is_string())
            .collect();
        ensure!(
            preferred.len() == 1,
            "ambiguous XP3 inline preferred name: {name}"
        );
        let winner = preferred[0].id;
        let shadowed: Vec<_> = entries
            .iter()
            .filter(|e| e.id != winner)
            .map(|e| e.id)
            .collect();
        hidden.extend(shadowed.iter().copied());
        shadows.insert(winner, shadowed);
    }
    // Native info aliases must not silently override another exposed name.
    for (alias, id) in &aliases {
        if let Some(entries) = names.get(alias) {
            ensure!(
                entries
                    .iter()
                    .filter(|e| !hidden.contains(&e.id))
                    .all(|e| e.id == *id),
                "XP3 info alias conflicts with an exposed resource"
            );
        }
    }
    let visible = if hidden.is_empty() {
        None
    } else {
        Some(
            inner
                .entries()
                .iter()
                .filter(|e| !hidden.contains(&e.id))
                .map(|e| {
                    let mut e = e.clone();
                    if let Some(ids) = shadows.get(&e.id) {
                        e.metadata["xp3_shadowed_native_ids"] = serde_json::json!(ids);
                    }
                    e
                })
                .collect(),
        )
    };
    Ok(Box::new(InlineArchive {
        inner,
        visible,
        aliases,
    }))
}
struct InlineArchive {
    inner: Box<dyn Archive>,
    visible: Option<Vec<ArchiveEntry>>,
    aliases: BTreeMap<String, u32>,
}
impl Archive for InlineArchive {
    fn entries(&self) -> &[ArchiveEntry] {
        self.visible
            .as_deref()
            .unwrap_or_else(|| self.inner.entries())
    }
    fn resolve(&self, path: &str) -> Option<u32> {
        self.aliases
            .get(path)
            .copied()
            .or_else(|| self.inner.resolve(path))
    }
    fn open(&self, id: u32, ctx: &DecodeContext) -> Result<Arc<dyn AssetReader>> {
        self.inner.open(id, ctx)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::{service::MemoryAssetReader, state::ClientId};
    struct Source(Vec<ArchiveEntry>);
    impl Archive for Source {
        fn entries(&self) -> &[ArchiveEntry] {
            &self.0
        }
        fn open(&self, id: u32, _: &DecodeContext) -> Result<Arc<dyn AssetReader>> {
            Ok(Arc::new(MemoryAssetReader(Arc::from([id as u8]))))
        }
    }
    fn entry(id: u32, path: &str, inline: bool) -> ArchiveEntry {
        ArchiveEntry {
            id,
            path: path.into(),
            size: 1,
            stored_size: 1,
            metadata: if inline {
                serde_json::json!({"xp3_inline_name":path,"xp3_info_name":format!("native_token_{id}")})
            } else {
                serde_json::json!({})
            },
        }
    }
    #[test]
    fn explicit_inline_namespace_wins_over_fallback_without_losing_original_record_or_info_alias() {
        let a = publish(Box::new(Source(vec![
            entry(0, "startup.tjs", false),
            entry(1, "startup.tjs", true),
            entry(2, "scene.ks", true),
        ])))
        .unwrap();
        assert_eq!(a.entries().len(), 2);
        assert_eq!(a.entries()[0].id, 1);
        assert_eq!(
            a.entries()[0].metadata["xp3_shadowed_native_ids"],
            serde_json::json!([0])
        );
        assert_eq!(a.resolve("native_token_1"), Some(1));
        let cx = DecodeContext::new(ClientId(1));
        assert_eq!(read_all(a.open(0, &cx).unwrap().as_ref(), 10).unwrap(), [0]);
        assert_eq!(read_all(a.open(1, &cx).unwrap().as_ref(), 10).unwrap(), [1]);
        let mount = crysirror_resource::ArchiveMount::new(a, cx).unwrap();
        assert_eq!(
            read_all(mount.open_entry("startup.tjs").unwrap().as_ref(), 10).unwrap(),
            [1]
        );
        assert_eq!(
            read_all(mount.open_entry("native_token_1").unwrap().as_ref(), 10).unwrap(),
            [1]
        );
    }
    #[test]
    fn multiple_inline_records_or_token_conflicts_are_not_guessed() {
        assert!(publish(Box::new(Source(vec![
            entry(0, "same", true),
            entry(1, "same", true)
        ])))
        .is_err());
        assert!(publish(Box::new(Source(vec![
            entry(0, "native_token_1", false),
            entry(1, "other", true)
        ])))
        .is_err());
    }
}
