// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! A capability-scoped loose-file view ahead of a read-only provider. This
//! selects actual sources at mount lookup, never by retrying a failed decoder.
use super::*;
pub(super) struct LooseFirst {
    asset: Dir,
    point: String,
    provider: Arc<dyn VfsProvider>,
}
impl LooseFirst {
    pub(super) fn new(asset: Dir, point: String, provider: Arc<dyn VfsProvider>) -> Self {
        Self {
            asset,
            point,
            provider,
        }
    }
    fn path(&self, relative: &str) -> Result<String> {
        let path = match (self.point.is_empty(), relative.is_empty()) {
            (true, _) => relative.to_owned(),
            (false, true) => self.point.clone(),
            (false, false) => format!("{}/{relative}", self.point),
        };
        if !path.is_empty() && VfsPath::new(Mount::Asset, &path)?.relative() != path {
            return Err(Error::Invalid("fallback path must be canonical"));
        }
        Ok(path)
    }
    fn metadata(&self, path: &str) -> Result<Option<cap_std::fs::Metadata>> {
        match self
            .asset
            .metadata(if path.is_empty() { "." } else { path })
        {
            Ok(value) => Ok(Some(value)),
            Err(reason)
                if reason.kind() == std::io::ErrorKind::NotFound
                    || !self.point.is_empty()
                        && reason.kind() == std::io::ErrorKind::NotADirectory =>
            {
                Ok(None)
            }
            Err(reason) => Err(io(reason)),
        }
    }
}
impl VfsProvider for LooseFirst {
    fn kind(&self) -> &'static str {
        self.provider.kind()
    }
    fn container_source(&self) -> Option<&str> {
        self.provider.container_source()
    }
    fn container_source_for(&self, relative: &str) -> Option<&str> {
        let path = self.path(relative).ok()?;
        match self.metadata(&path) {
            Ok(None) => self.provider.container_source_for(relative),
            _ => None,
        }
    }
    fn read(&self, relative: &str) -> Result<Vec<u8>> {
        let path = self.path(relative)?;
        match self.metadata(&path)? {
            Some(metadata) if metadata.is_file() => {
                read_limited_virtual(&self.asset, &path, !self.point.is_empty())
            }
            Some(_) => Err(Error::Invalid("fallback source must be a regular file")),
            None => self.provider.read(relative),
        }
    }
    fn open(&self, relative: &str) -> Result<Arc<dyn AssetReader>> {
        let path = self.path(relative)?;
        let Some(metadata) = self.metadata(&path)? else {
            return self.provider.open(relative);
        };
        // Directory queries are handled by list_dir/metadata, never as files.
        if metadata.is_dir() {
            return Err(Error::NotFound);
        }
        if !metadata.is_file() {
            return Err(Error::Unsupported("special fallback asset"));
        }
        let file = self.asset.open(&path).map_err(io)?;
        let metadata = file.metadata().map_err(io)?;
        if !metadata.is_file() {
            return Err(Error::Invalid("fallback stream must be a file"));
        }
        Ok(Arc::new(CapAssetReader {
            file: Mutex::new(file),
            len: metadata.len(),
        }))
    }
    fn exists(&self, relative: &str) -> bool {
        let Ok(path) = self.path(relative) else {
            return false;
        };
        match self.metadata(&path) {
            Ok(Some(metadata)) => metadata.is_file(),
            Ok(None) => self.provider.exists(relative),
            Err(_) => false,
        }
    }
    fn list_dir(&self, relative: &str) -> Result<Vec<String>> {
        let path = self.path(relative)?;
        let mut names = std::collections::BTreeSet::new();
        let entries = if path.is_empty() {
            self.asset.entries()
        } else {
            self.asset.read_dir(&path)
        };
        match entries {
            Ok(entries) => {
                for entry in entries {
                    if let Ok(name) = entry.map_err(io)?.file_name().into_string() {
                        if !name.starts_with(".crysirror-") {
                            names.insert(name);
                        }
                    }
                    if names.len() > 65536 {
                        return Err(Error::Limit("fallback directory listing"));
                    }
                }
            }
            Err(reason)
                if reason.kind() == std::io::ErrorKind::NotFound
                    || reason.kind() == std::io::ErrorKind::NotADirectory =>
            {
                ()
            }
            Err(reason) => return Err(io(reason)),
        }
        names.extend(self.provider.list_dir(relative)?);
        if names.len() > 65536 {
            return Err(Error::Limit("fallback directory listing"));
        }
        Ok(names.into_iter().collect())
    }
}
