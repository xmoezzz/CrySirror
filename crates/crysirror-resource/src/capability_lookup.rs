// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Bounded name resolution within an already client-scoped storage capability.
use anyhow::{Result, ensure};
use crysirror_core::{path::VfsPath, service::ClientStorage};

/// Resolve canonical components using ASCII case-insensitive directory names.
/// This only observes directory metadata: it never opens a payload, creates a
/// host path, chooses a resource extension or escapes the supplied capability.
/// Missing components return `None`; ambiguous names and excessive depth or
/// aggregate directory entries fail explicitly. The final name may be a file
/// or directory; the caller's resource/open policy retains that distinction.
pub fn resolve_ascii_case_insensitive(
    storage: &dyn ClientStorage,
    path: &VfsPath,
) -> Result<Option<VfsPath>> {
    let components = path.relative().split('/').collect::<Vec<_>>();
    ensure!(components.len() <= 64, "capability lookup depth budget");
    let mut resolved = String::new();
    let mut entries = 0usize;
    for component in components {
        let names = storage.list(path.mount(), &resolved)?;
        entries = entries
            .checked_add(names.len())
            .ok_or_else(|| anyhow::anyhow!("capability lookup entry overflow"))?;
        ensure!(entries <= 65536, "capability lookup entry budget");
        let mut matches = names
            .into_iter()
            .filter(|n| n.eq_ignore_ascii_case(component));
        let Some(name) = matches.next() else {
            return Ok(None);
        };
        ensure!(
            matches.next().is_none(),
            "ambiguous ASCII case-insensitive capability name"
        );
        if !resolved.is_empty() {
            resolved.push('/');
        }
        resolved.push_str(&name);
        // A provider cannot turn a direct child into traversal/another mount.
        VfsPath::new(path.mount(), &resolved)?;
    }
    Ok(Some(VfsPath::new(path.mount(), &resolved)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::{Error, path::Mount};
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct Storage(BTreeMap<String, Vec<String>>);
    impl ClientStorage for Storage {
        fn read(&self, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
            panic!("name lookup must not read payloads")
        }
        fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
            panic!("read-only lookup")
        }
        fn remove(&self, _: &VfsPath) -> crysirror_core::Result<()> {
            panic!("read-only lookup")
        }
        fn list(&self, mount: Mount, dir: &str) -> crysirror_core::Result<Vec<String>> {
            if mount != Mount::Asset {
                return Err(Error::ReadOnly);
            }
            self.0.get(dir).cloned().ok_or(Error::NotFound)
        }
    }
    #[test]
    fn resolves_only_names_in_the_supplied_capability_and_preserves_actual_case() {
        let storage = Storage(BTreeMap::from([
            ("".into(), vec!["Wav".into()]),
            ("Wav".into(), vec!["MN_SS_SE31.nwa".into()]),
        ]));
        assert_eq!(
            resolve_ascii_case_insensitive(
                &storage,
                &VfsPath::new(Mount::Asset, "wav/mn_ss_se31.NWA").unwrap()
            )
            .unwrap()
            .unwrap()
            .relative(),
            "Wav/MN_SS_SE31.nwa"
        );
        assert!(
            resolve_ascii_case_insensitive(
                &storage,
                &VfsPath::new(Mount::Asset, "wav/missing.nwa").unwrap()
            )
            .unwrap()
            .is_none()
        );
        assert!(
            resolve_ascii_case_insensitive(
                &Storage::default(),
                &VfsPath::new(Mount::Asset, "wav/mn_ss_se31.NWA").unwrap()
            )
            .is_err()
        );
        assert!(VfsPath::new(Mount::Asset, "../Wav/MN_SS_SE31.nwa").is_err());
    }
    #[test]
    fn ambiguous_names_and_budgets_fail_without_opening_files() {
        let storage = Storage(BTreeMap::from([("".into(), vec!["a".into(), "A".into()])]));
        assert!(
            resolve_ascii_case_insensitive(&storage, &VfsPath::new(Mount::Asset, "a").unwrap())
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        let storage = Storage(BTreeMap::from([("".into(), vec!["other".into(); 65537])]));
        assert!(
            resolve_ascii_case_insensitive(&storage, &VfsPath::new(Mount::Asset, "a").unwrap())
                .unwrap_err()
                .to_string()
                .contains("entry budget")
        );
        let path = std::iter::repeat_n("a", 65).collect::<Vec<_>>().join("/");
        assert!(
            resolve_ascii_case_insensitive(
                &Storage::default(),
                &VfsPath::new(Mount::Asset, &path).unwrap()
            )
            .unwrap_err()
            .to_string()
            .contains("depth budget")
        );
    }
}
