// SPDX-License-Identifier: AGPL-3.0-or-later
//! KiriKiri storage naming over the client's engine-owned capability storage.
//!
//! KiriKiri resolves bare storage names through an ordered "auto path" list
//! with case-insensitive matching. Only that naming policy lives here; bytes
//! come from the shared `ClientStorage` handle and images are decoded by the
//! engine's shared decoders.
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::ClientStorage,
};
use std::{collections::BTreeMap, sync::Arc};

/// Upper bound on indexed files; a project larger than this fails visibly.
const MAX_FILES: usize = 65_536;

pub struct Storage {
    files: Arc<dyn ClientStorage>,
    root: String,
    current_uri: String,
    /// Lowercased path relative to `root` → provider spelling relative to the
    /// asset mount.
    index: BTreeMap<String, String>,
    /// Lowercased auto-path directories, searched after a direct match.
    auto: Vec<String>,
    /// Registered compound media (`arc://`), owned by the shared resource service.
    pub compound: Option<(String, crysirror_resource::compound::CompoundStorage)>,
    /// Directory inside each routed container that answers hashed names.
    pub hashed_root: Option<String>,
}

impl Storage {
    fn scoped_location(&self, name: &str) -> Result<(Mount, String)> {
        let full = self.full_name(name)?;
        ensure!(!full.contains("://"), "compound metadata is not supported");
        let relative = full.trim_end_matches('/');
        if relative == "savedata" { return Ok((Mount::Save, String::new())); }
        if let Some(save) = relative.strip_prefix("savedata/") {
            return Ok((Mount::Save, VfsPath::new(Mount::Save, save).map_err(err)?.relative().to_owned()));
        }
        let asset = if self.root.is_empty() { relative.to_owned() } else if relative.is_empty() { self.root.clone() } else { format!("{}/{relative}", self.root) };
        if !asset.is_empty() { VfsPath::new(Mount::Asset, &asset).map_err(err)?; }
        Ok((Mount::Asset, asset))
    }
    pub fn metadata(&self, name: &str) -> Result<Option<crysirror_core::service::StorageMetadata>> {
        let (mount, relative) = self.scoped_location(name)?;
        self.files.metadata(mount, &relative).map_err(err)
    }
    pub fn directory_entries(&self, name: &str) -> Result<Vec<String>> {
        let (mount, relative) = self.scoped_location(name)?;
        ensure!(self.files.metadata(mount, &relative).map_err(err)?.is_some_and(|m| m.is_directory), "Directory not found.");
        let mut result = Vec::new();
        for mut child in self.files.list(mount, &relative).map_err(err)? {
            let full = if relative.is_empty() { child.clone() } else { format!("{relative}/{child}") };
            if self.files.metadata(mount, &full).map_err(err)?.is_some_and(|m| m.is_directory) { child.push('/'); }
            result.push(child);
        }
        Ok(result)
    }

    pub fn new(files: Arc<dyn ClientStorage>, root: &str) -> Result<Self> {
        let mut this = Self {
            files,
            root: root.trim_matches('/').to_owned(),
            current_uri: "file://./".into(),
            index: BTreeMap::new(),
            auto: Vec::new(),
            compound: None,
            hashed_root: None,
        };
        let mut stack = vec![this.root.clone()];
        while let Some(dir) = stack.pop() {
            for name in this.files.list(Mount::Asset, &dir).map_err(err)? {
                let full = if dir.is_empty() {
                    name.clone()
                } else {
                    format!("{dir}/{name}")
                };
                // Providers list directories and files alike; a directory has
                // children, a file reads. Probe by listing first.
                match this.files.list(Mount::Asset, &full) {
                    Ok(children) if !children.is_empty() => stack.push(full),
                    _ => {
                        let relative = full
                            .strip_prefix(&this.root)
                            .unwrap_or(&full)
                            .trim_start_matches('/')
                            .to_lowercase();
                        this.index.insert(relative, full);
                        ensure!(
                            this.index.len() <= MAX_FILES,
                            "KiriKiri storage index limit"
                        );
                    }
                }
            }
        }
        Ok(this)
    }

    /// Qualify native names without allowing host paths or arbitrary media.
    pub fn full_uri(&self, name: &str) -> Result<String> {
        let name = name.replace('\\', "/");
        if let Some(rest) = self.compound_relative(&name) {
            return Ok(format!("{}://./{}", self.compound_media().unwrap(), rest?));
        }
        if !name.contains(':') && !name.starts_with('/') {
            if let Some(media) = self.compound_media() {
                if let Some(base) = self.current_uri.strip_prefix(&format!("{media}://./")) {
                    let joined = format!("{media}://./{base}{name}");
                    return Ok(format!("{media}://./{}", self.compound_relative(&joined).unwrap()?));
                }
            }
        }
        ensure!(
            !name.contains(':') || name.starts_with("file://./"),
            "unsupported native storage media/domain"
        );
        let uri = if name.starts_with("file://./") {
            name
        } else if name.starts_with('/') {
            format!("file://./{}", name.trim_start_matches('/'))
        } else {
            format!("{}{name}", self.current_uri)
        };
        let body = uri
            .strip_prefix("file://./")
            .context("native file domain")?;
        ensure!(
            body.matches('>').count() <= 1,
            "nested native archive media unsupported"
        );
        let normalize_part = |part: &str| -> Result<String> {
            if part.is_empty() {
                return Ok(String::new());
            }
            let dir = part.ends_with('/');
            let n = normalize(part);
            if n.is_empty() {
                return Ok(String::new());
            }
            let path = VfsPath::new(Mount::Asset, &n).map_err(err)?;
            Ok(format!("{}{}", path.relative(), if dir { "/" } else { "" }))
        };
        if let Some((container, inner)) = body.split_once('>') {
            let container = normalize_part(container)?;
            ensure!(
                !container.is_empty() && !container.ends_with('/'),
                "native archive source"
            );
            Ok(format!("file://./{container}>{}", normalize_part(inner)?))
        } else {
            Ok(format!("file://./{}", normalize_part(body)?))
        }
    }
    fn compound_media(&self) -> Option<&str> {
        self.compound.as_ref().map(|(m, _)| m.as_str())
    }
    /// Canonical domain-relative path of a compound-media URI (`arc://./x`).
    fn compound_relative(&self, name: &str) -> Option<Result<String>> {
        let media = self.compound_media()?;
        let rest = name.strip_prefix(media)?.strip_prefix("://")?;
        Some((|| {
            let rest = rest
                .strip_prefix("./")
                .context("compound storage supports only the root domain `.`")?;
            let dir = rest.is_empty() || rest.ends_with('/');
            let n = normalize(rest);
            if n.is_empty() {
                return Ok(String::new());
            }
            let path = VfsPath::new(Mount::Asset, &n).map_err(err)?;
            Ok(format!("{}{}", path.relative(), if dir { "/" } else { "" }))
        })())
    }
    /// Asset-relative point where shared container `source` is mounted.
    pub fn container_point(&self, source: &str) -> Result<Option<String>> {
        let source = if self.root.is_empty() {
            source.to_owned()
        } else {
            format!("{}/{source}", self.root)
        };
        let Some(point) = self.files.container_mount(&source).map_err(err)? else {
            return Ok(None);
        };
        let relative = if self.root.is_empty() {
            point.as_str()
        } else if point == self.root {
            ""
        } else {
            point
                .strip_prefix(&format!("{}/", self.root))
                .context("container mount outside native root")?
        };
        Ok(Some(relative.to_owned()))
    }
    /// Resolve a domain-relative compound path through its routed containers.
    fn resolve_compound(&self, relative: &str) -> Option<String> {
        let (_, media) = self.compound.as_ref()?;
        for candidate in media.candidates(".", relative) {
            let Ok(Some(point)) = self.container_point(&candidate.container) else {
                continue;
            };
            for inner in [candidate.inner.clone(), candidate.inner.to_lowercase()] {
                let inner = match &self.hashed_root {
                    Some(root) => format!("{root}/{inner}"),
                    None => inner,
                };
                let path = match (self.root.is_empty(), point.is_empty()) {
                    (true, true) => inner,
                    (true, false) => format!("{point}/{inner}"),
                    (false, true) => format!("{}/{inner}", self.root),
                    (false, false) => format!("{}/{point}/{inner}", self.root),
                };
                let Ok(checked) = VfsPath::new(Mount::Asset, &path) else {
                    continue;
                };
                if self.files.exists(&checked) {
                    return Some(path);
                }
            }
        }
        None
    }
    /// Container source of a plain `file://` archive name that is mounted.
    pub fn archive_source(&self, name: &str) -> Result<Option<String>> {
        let uri = self.full_uri(name)?;
        let Some(body) = uri.strip_prefix("file://./") else {
            return Ok(None);
        };
        ensure!(!body.contains('>') && !body.ends_with('/'), "archive file name");
        Ok(self.container_point(body)?.map(|_| body.to_owned()))
    }
    /// Existence of exactly this storage name: no auto-path search.
    pub fn exists_exact(&self, name: &str) -> bool {
        let Ok(uri) = self.full_uri(name) else {
            return false;
        };
        if let Some(Ok(rel)) = self.compound_relative(&uri) {
            return self.resolve_compound(&rel).is_some();
        }
        let Some(body) = uri.strip_prefix("file://./") else {
            return false;
        };
        if !body.contains('>') && matches!(self.container_point(body), Ok(Some(_))) {
            return true;
        }
        let Ok(path) = self.full_name(&uri) else {
            return false;
        };
        let path = if self.root.is_empty() { path } else { format!("{}/{path}", self.root) };
        VfsPath::new(Mount::Asset, &path).is_ok_and(|p| self.files.exists(&p))
    }
    /// Resolve native container syntax only to an already mounted shared provider.
    pub fn full_name(&self, name: &str) -> Result<String> {
        let uri = self.full_uri(name)?;
        if self.compound_relative(&uri).is_some() {
            // Compound media are routed per lookup; keep the logical URI.
            return Ok(uri);
        }
        let body = uri.strip_prefix("file://./").unwrap();
        if let Some((container, inner)) = body.split_once('>') {
            let relative = self
                .container_point(container)?
                .context("native archive URI requires an already mounted shared container")?;
            let relative = relative.as_str();
            Ok(if relative.is_empty() {
                inner.to_owned()
            } else if inner.is_empty() {
                format!("{relative}/")
            } else {
                format!("{relative}/{inner}")
            })
        } else {
            Ok(body.to_owned())
        }
    }
    pub fn set_default_path(&mut self, name: &str) -> Result<()> {
        ensure!(
            name.ends_with(['/', '\\', '>']),
            "native default directory requires trailing delimiter"
        );
        let full = self.full_name(name)?;
        ensure!(
            !full.starts_with("savedata/"),
            "save-media default directory is not yet mapped"
        );
        if let Some(rel) = self.compound_relative(&full) {
            ensure!(rel?.is_empty() || full.ends_with('/'), "compound default directory");
        }
        self.current_uri = self.full_uri(name)?;
        Ok(())
    }
    /// An archive auto path whose container file does not exist at all: the
    /// native engine keeps such a path, and lookups through it find nothing.
    fn absent_archive(&self, path: &str) -> bool {
        let Ok(uri) = self.full_uri(path) else {
            return false;
        };
        let Some((container, _)) = uri.strip_prefix("file://./").and_then(|b| b.split_once('>')) else {
            return false;
        };
        if matches!(self.container_point(container), Ok(Some(_))) {
            return false;
        }
        let source = if self.root.is_empty() {
            container.to_owned()
        } else {
            format!("{}/{container}", self.root)
        };
        !VfsPath::new(Mount::Asset, &source).is_ok_and(|p| self.files.exists(&p))
    }
    /// Auto paths reuse explicit engine container mounts; they never open archives.
    pub fn add_auto_path(&mut self, path: &str) -> Result<()> {
        if self.absent_archive(path) {
            return Ok(());
        }
        let dir = self.full_name(path)?.trim_end_matches('/').to_lowercase();
        if !self.auto.contains(&dir) {
            self.auto.push(dir)
        }
        Ok(())
    }
    pub fn remove_auto_path(&mut self, path: &str) -> Result<()> {
        if self.absent_archive(path) {
            return Ok(());
        }
        let dir = self.full_name(path)?.trim_end_matches('/').to_lowercase();
        self.auto.retain(|d| *d != dir);
        Ok(())
    }

    /// Resolve a storage name to an asset-relative path, or `None`.
    pub fn resolve(&self, name: &str) -> Option<String> {
        if let Ok(Some(path)) = save_path(name) {
            return self
                .files
                .exists(&path)
                .then(|| format!("savedata/{}", path.relative()));
        }
        let normalized = self.full_name(name).ok()?;
        if let Some(rel) = self.compound_relative(&normalized) {
            let rel = rel.ok()?;
            if let Some(found) = self.resolve_compound(&rel) {
                return Some(found);
            }
            let base = rel.rsplit('/').next().unwrap_or(&rel).to_owned();
            return self.resolve_auto(&base);
        }
        let key = normalized.to_lowercase();
        if key.is_empty() {
            return None;
        }
        if let Some(found) = self.index.get(&key) {
            return Some(found.clone());
        }
        let base = key.rsplit('/').next().unwrap_or(&key);
        for dir in self.auto.iter().rev() {
            let candidate = if dir.is_empty() {
                base.to_owned()
            } else {
                format!("{dir}/{base}")
            };
            if let Some(found) = self.index.get(&candidate) {
                return Some(found.clone());
            }
        }
        // Hash-indexed engine archive providers may resolve a requested spelling
        // without being able to enumerate plaintext names. Keep the same root.
        let candidate = |name: &str| {
            let path = if self.root.is_empty() {
                name.to_owned()
            } else {
                format!("{}/{name}", self.root)
            };
            let checked = VfsPath::new(Mount::Asset, &path).ok()?;
            self.files.exists(&checked).then_some(path)
        };
        if let Some(path) = candidate(&normalized) {
            return Some(path);
        }
        if let Some(path) = candidate(&key) {
            return Some(path);
        }
        for dir in self.auto.iter().rev() {
            if let Some(path) = candidate(&format!("{dir}/{base}")) {
                return Some(path);
            }
        }
        self.resolve_auto(base)
    }
    /// Auto-path directories inside compound media.
    fn resolve_auto(&self, base: &str) -> Option<String> {
        for dir in self.auto.iter().rev() {
            if let Some(Ok(rel)) = self.compound_relative(dir) {
                let rel = rel.trim_end_matches('/');
                let path = if rel.is_empty() { base.to_owned() } else { format!("{rel}/{base}") };
                if let Some(found) = self.resolve_compound(&path) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Resolve trying KiriKiri's implicit extensions in order.
    pub fn resolve_with(&self, name: &str, extensions: &[&str]) -> Option<String> {
        if let Some(found) = self.resolve(name) {
            return Some(found);
        }
        extensions
            .iter()
            .find_map(|ext| self.resolve(&format!("{name}{ext}")))
    }

    pub fn begin_write(
        &self,
        name: &str,
        data: std::sync::Arc<[u8]>,
    ) -> Result<Box<dyn crysirror_core::service::StorageWrite>> {
        let path =
            save_path(name)?.context("writes require the virtual savedata/ capability root")?;
        self.files.begin_write_atomic(&path, data).map_err(err)
    }
    /// Enforce the byte limit before allocating or reading the stream.
    pub fn read_limited(&self, path: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let path =
            save_path(path)?.map_or_else(|| VfsPath::new(Mount::Asset, path).map_err(err), Ok)?;
        let reader = self.files.open_read(&path).map_err(err)?;
        let len = reader.len().map_err(err)?;
        ensure!(len <= max_bytes as u64, "native storage byte limit");
        let mut out = vec![0; len as usize];
        let mut at = 0;
        while at < out.len() {
            let end = (at + 65536).min(out.len());
            let n = reader.read_at(at as u64, &mut out[at..end]).map_err(err)?;
            ensure!(n > 0 && n <= end - at, "short native storage read");
            at += n;
        }
        Ok(out)
    }
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        if let Some(path) = save_path(path)? {
            return self.files.read(&path).map_err(err);
        }
        self.files
            .read(&VfsPath::new(Mount::Asset, path).map_err(err)?)
            .map_err(err)
            .with_context(|| format!("read storage `{path}`"))
    }

    pub fn read_named(&self, name: &str) -> Result<Vec<u8>> {
        if let Some(path) = save_path(name)? {
            return self.files.read(&path).map_err(err);
        }
        let path = self
            .resolve(name)
            .with_context(|| format!("storage `{name}` not found"))?;
        if std::env::var_os("CRYSIRROR_KRKR_TRACE_STORAGE").is_some() {
            eprintln!("[storage read] {name} -> {path}");
        }
        self.read(&path)
    }
}

/// Only this virtual prefix grants native writes; it never denotes an OS path.
pub fn save_path(name: &str) -> Result<Option<VfsPath>> {
    let normalized = normalize(name);
    let Some(relative) = normalized.strip_prefix("savedata/") else {
        return Ok(None);
    };
    let relative = relative
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect::<Vec<_>>()
        .join("/");
    Ok(Some(VfsPath::new(Mount::Save, &relative).map_err(err)?))
}

/// Strip KiriKiri's `file://./` and archive prefixes into a slash path.
pub fn normalize(name: &str) -> String {
    let mut s = name.replace('\\', "/");
    for prefix in ["file://./", "file://", "./"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_owned();
        }
    }
    s.trim_start_matches('/').to_owned()
}

/// Decode a script text the way KiriKiri does: a UTF-16LE/UTF-8 BOM wins,
/// then valid UTF-8, then Shift_JIS (the platform default of most titles).
pub fn decode_text(bytes: &[u8]) -> Result<String> {
    if let Some(text) = crysirror_resource::text_stream::decode(bytes, 64 * 1024 * 1024)? {
        return Ok(text);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        ensure!(rest.len() % 2 == 0, "odd UTF-16LE script length");
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16(&units).context("invalid UTF-16LE script");
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec()).context("invalid UTF-8 script");
    }
    ensure!(
        !bytes.starts_with(b"TJS2"),
        "compiled TJS2 bytecode must be executed, not decoded as text"
    );
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(text.to_owned());
    }
    let (text, _, errors) = encoding_rs::SHIFT_JIS.decode(bytes);
    ensure!(!errors, "script is neither UTF-8 nor Shift_JIS");
    Ok(text.into_owned())
}

fn err(e: crysirror_core::Error) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_prefixes_and_decodes_boms() {
        assert_eq!(normalize("file://./data\\bg.png"), "data/bg.png");
        assert_eq!(decode_text(&[0xFF, 0xFE, b'a', 0, b'b', 0]).unwrap(), "ab");
        assert_eq!(decode_text(b"\xEF\xBB\xBFok").unwrap(), "ok");
        let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode("吉里吉里");
        assert_eq!(decode_text(&sjis).unwrap(), "吉里吉里");
    }
}
