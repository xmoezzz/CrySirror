// SPDX-License-Identifier: MPL-2.0
//! RealLive resource naming policy over engine-owned capability storage.
use crate::vm::Gameexe;
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::{
    Error,
    path::{Mount, VfsPath},
    service::ClientStorage,
};
use std::{collections::BTreeMap, sync::Arc};
pub struct Resources {
    pub storage: Arc<dyn ClientStorage>,
    pub root: String,
    dirs: BTreeMap<String, Vec<String>>,
}
impl Resources {
    pub fn new(storage: Arc<dyn ClientStorage>, root: String) -> Self {
        Self {
            storage,
            root,
            dirs: BTreeMap::new(),
        }
    }
    pub fn join(&self, dir: &str) -> String {
        if self.root.is_empty() {
            dir.into()
        } else if dir.is_empty() {
            self.root.clone()
        } else {
            format!("{}/{dir}", self.root)
        }
    }
    /// Resolve every component through the scoped provider, including directory
    /// case. Returned names retain the provider's spelling on case-sensitive OSes.
    pub fn file(&mut self, dir: &str, name: &str) -> Result<Option<String>> {
        Self::component(name)?;
        let dir = if dir.is_empty() || dir == "." {
            String::new()
        } else {
            VfsPath::new(Mount::Asset, dir)?.relative().to_owned()
        };
        let joined = self.join(&dir);
        let relative = if joined.is_empty() {
            String::new()
        } else {
            VfsPath::new(Mount::Asset, &joined)?.relative().to_owned()
        };
        let requested = if relative.is_empty() {
            name.into()
        } else {
            format!("{relative}/{name}")
        };
        let requested = VfsPath::new(Mount::Asset, &requested)?;
        let mut actual = String::new();
        for component in requested.relative().split('/') {
            let Some(entry) = self.entry(&actual, component)? else {
                return Ok(None);
            };
            if !actual.is_empty() {
                actual.push('/');
            }
            actual.push_str(&entry);
        }
        Ok(Some(actual))
    }
    fn component(name: &str) -> Result<()> {
        let path = VfsPath::new(Mount::Asset, name)?;
        ensure!(
            path.relative() == name && !name.contains('/'),
            "resource entry must be one canonical component"
        );
        Ok(())
    }
    fn entry(&mut self, dir: &str, name: &str) -> Result<Option<String>> {
        if !self.dirs.contains_key(dir) {
            let list = match self.storage.list(Mount::Asset, dir) {
                Ok(v) => v,
                Err(Error::NotFound) => return Ok(None),
                Err(e) => {
                    return Err(anyhow::Error::new(e).context(format!("resource directory {dir}")));
                }
            };
            for entry in &list {
                Self::component(entry)
                    .with_context(|| format!("invalid provider entry in {dir}"))?;
            }
            self.dirs.insert(dir.into(), list);
        }
        let lower = name.to_lowercase();
        let mut matches = self.dirs[dir].iter().filter(|n| n.to_lowercase() == lower);
        let found = matches.next().cloned();
        ensure!(
            matches.next().is_none(),
            "ambiguous case-insensitive resource {dir}/{name}"
        );
        Ok(found)
    }
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        self.storage
            .read(&VfsPath::new(Mount::Asset, path)?)
            .with_context(|| format!("resource {path}"))
    }
    pub fn find(&mut self, exe: &Gameexe, kind: &str, name: &str) -> Result<String> {
        anyhow::ensure!(
            !name.contains("..") && !name.contains(':') && !name.starts_with('/'),
            "invalid native resource name"
        );
        let (folders, extensions): (&[&str], &[&str]) = match kind {
            "image" => (&["G00", "PDT"], &["g00", "pdt", "png", "bmp", "jpg"]),
            "bgm" => (&["BGM"], &["nwa", "ogg", "wav", "mp3", "owp"]),
            "wav" => (&["WAV"], &["wav", "nwa", "ogg", "owp"]),
            "koe" => (&["KOE"], &["koe", "ovk", "nwk", "ogg", "wav", "nwa"]),
            "movie" => (&["MOV"], &["mpg", "mpeg", "avi", "wmv", "ogv", "mp4"]),
            _ => bail!("unknown resource kind"),
        };
        let name = name.replace('\\', "/");
        VfsPath::new(Mount::Asset, &name)?;
        let mut dirs = Vec::new();
        for folder in folders {
            if let Some(e) = exe.get(&format!("FOLDNAME.{folder}")) {
                if let Some(d) = e.str(0) {
                    dirs.push(d.replace('\\', "/"));
                }
            } else {
                dirs.push(folder.to_lowercase());
            }
        }
        dirs.push(String::new());
        let candidates = if name.rsplit('/').next().unwrap_or("").contains('.') {
            vec![name.clone()]
        } else {
            extensions.iter().map(|e| format!("{name}.{e}")).collect()
        };
        for dir in &dirs {
            for candidate in &candidates {
                let (sub, filename) = candidate.rsplit_once('/').unwrap_or(("", candidate));
                let folder = if sub.is_empty() {
                    dir.clone()
                } else if dir.is_empty() {
                    sub.into()
                } else {
                    format!("{dir}/{sub}")
                };
                if let Some(path) = self.file(&folder, filename)? {
                    return Ok(path);
                }
            }
        }
        Err(anyhow::anyhow!(
            "RealLive {kind} resource '{name}' not found"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Storage {
        files: BTreeMap<String, Vec<u8>>,
        lists: Mutex<Vec<String>>,
        list_error: Option<Error>,
        entries: Option<Vec<String>>,
    }
    impl Storage {
        fn with_file(path: &str, bytes: &[u8]) -> Self {
            Self {
                files: BTreeMap::from([(path.into(), bytes.into())]),
                ..Default::default()
            }
        }
    }
    impl ClientStorage for Storage {
        fn read(&self, path: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
            if path.mount() != Mount::Asset {
                return Err(Error::ReadOnly);
            }
            self.files
                .get(path.relative())
                .cloned()
                .ok_or(Error::NotFound)
        }
        fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
            Err(Error::ReadOnly)
        }
        fn remove(&self, _: &VfsPath) -> crysirror_core::Result<()> {
            Err(Error::ReadOnly)
        }
        fn list(&self, mount: Mount, dir: &str) -> crysirror_core::Result<Vec<String>> {
            assert_eq!(mount, Mount::Asset);
            self.lists.lock().unwrap().push(dir.into());
            if let Some(error) = &self.list_error {
                return Err(error.clone());
            }
            if let Some(entries) = &self.entries {
                return Ok(entries.clone());
            }
            let prefix = if dir.is_empty() {
                String::new()
            } else {
                format!("{dir}/")
            };
            let mut entries = self
                .files
                .keys()
                .filter_map(|file| file.strip_prefix(&prefix))
                .filter_map(|file| file.split('/').next())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            entries.sort();
            entries.dedup();
            if entries.is_empty() && !dir.is_empty() {
                return Err(Error::NotFound);
            }
            Ok(entries)
        }
    }
    #[test]
    fn resolves_and_caches_root_directory_subdirectory_and_filename_case() {
        let storage = Arc::new(Storage::with_file("Game/PDT/Chapter/Title.PNG", b"image"));
        let mut resources = Resources::new(storage.clone(), "game".into());
        let path = resources.file("pdt/chapter", "title.png").unwrap().unwrap();
        assert_eq!(path, "Game/PDT/Chapter/Title.PNG");
        assert_eq!(resources.read(&path).unwrap(), b"image");
        assert_eq!(
            resources.file("PDT/CHAPTER", "TITLE.PNG").unwrap(),
            Some(path)
        );
        assert_eq!(
            *storage.lists.lock().unwrap(),
            ["", "Game", "Game/PDT", "Game/PDT/Chapter"]
        );
    }
    #[test]
    fn gameexe_folders_use_provider_case_and_preserve_relative_dot_prefix() {
        let storage = Arc::new(Storage::with_file("PDT/Chapter/Title.PNG", b"image"));
        let mut resources = Resources::new(storage, String::new());
        let exe = Gameexe::parse("#FOLDNAME.PDT=\".\\pdt\"\n");
        assert_eq!(
            resources.find(&exe, "image", "chapter\\title").unwrap(),
            "PDT/Chapter/Title.PNG"
        );
    }
    #[test]
    fn malicious_paths_are_rejected_without_storage_access() {
        for (root, dir, name) in [
            ("", "../other", "title.png"),
            ("", "/other", "title.png"),
            ("Game/../other", "", "title.png"),
            ("", "", "../title.png"),
            ("", "", "part/title.png"),
            ("", "", "C:secret"),
            ("", "", ".."),
            ("", "", "title\\image.png"),
        ] {
            let storage = Arc::new(Storage::with_file("Game/Title.PNG", b"image"));
            let mut resources = Resources::new(storage.clone(), root.into());
            assert!(resources.file(dir, name).is_err(), "{root}/{dir}/{name}");
            assert!(storage.lists.lock().unwrap().is_empty());
        }
    }
    #[test]
    fn gameexe_folder_traversal_is_rejected_instead_of_trimmed() {
        let storage = Arc::new(Storage::with_file("private/Title.PNG", b"secret"));
        let mut resources = Resources::new(storage.clone(), String::new());
        let exe = Gameexe::parse("#FOLDNAME.G00=\"../private\"\n");
        assert!(resources.find(&exe, "image", "title").is_err());
        assert!(storage.lists.lock().unwrap().is_empty());
        let exe = Gameexe::default();
        assert!(resources.find(&exe, "image", "\\private").is_err());
        assert!(storage.lists.lock().unwrap().is_empty());
    }
    #[test]
    fn missing_entries_are_optional_but_io_errors_keep_their_cause() {
        let mut resources = Resources::new(
            Arc::new(Storage::with_file("PDT/Title.PNG", b"image")),
            String::new(),
        );
        assert_eq!(resources.file("absent/subdir", "title.png").unwrap(), None);
        assert_eq!(resources.file("PDT", "absent.png").unwrap(), None);
        let mut resources = Resources::new(
            Arc::new(Storage {
                list_error: Some(Error::Io("permission denied".into())),
                ..Default::default()
            }),
            String::new(),
        );
        let error = resources.file("PDT", "title.png").unwrap_err();
        assert_eq!(
            error.downcast_ref::<Error>(),
            Some(&Error::Io("permission denied".into()))
        );
    }
    #[test]
    fn provider_entries_cannot_escape_and_ambiguous_case_fails() {
        for entries in [
            vec!["../private".into()],
            vec!["Title.PNG".into(), "title.png".into()],
        ] {
            let mut resources = Resources::new(
                Arc::new(Storage {
                    entries: Some(entries),
                    ..Default::default()
                }),
                String::new(),
            );
            assert!(resources.file("", "TITLE.PNG").is_err());
        }
    }
    #[test]
    fn resource_caches_and_reads_remain_client_scoped() {
        let mut first = Resources::new(
            Arc::new(Storage::with_file("PDT/Title.PNG", b"client one")),
            String::new(),
        );
        let mut second = Resources::new(
            Arc::new(Storage::with_file("pdt/title.png", b"client two")),
            String::new(),
        );
        let one = first.file("pdt", "title.png").unwrap().unwrap();
        let two = second.file("PDT", "TITLE.PNG").unwrap().unwrap();
        assert_eq!(first.read(&one).unwrap(), b"client one");
        assert_eq!(second.read(&two).unwrap(), b"client two");
        assert!(first.read(&two).is_err());
        assert!(second.read(&one).is_err());
    }
}
