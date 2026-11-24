// SPDX-License-Identifier: MPL-2.0
//! KiriKiri storage naming over the client's engine-owned capability storage.
//!
//! KiriKiri resolves bare storage names through an ordered "auto path" list
//! with case-insensitive matching. Only that naming policy lives here; bytes
//! come from the shared `ClientStorage` handle and images are decoded by the
//! engine's shared decoders.
// Native storage syntax does not grant filesystem authority. Auto paths and
// archive URIs resolve through installed client mounts; save names route to the
// Save capability. Native text encoding/offset modes remain stream policy rather
// than a host-path or archive-unpacking shortcut.
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::ClientStorage,
};
use std::{collections::BTreeMap, sync::Arc};

/// Upper bound on indexed files; a project larger than this fails visibly.
const MAX_FILES: usize = 65_536;

/// Read-only replacement archives selected by the installed-game launcher.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct LauncherOverlay {
    /// Mounted XP3 basenames, with the first existing member taking priority.
    pub archives: Vec<String>,
    /// Native media names eligible for exact member replacement.
    pub protocols: Vec<String>,
    /// Apply the same check to requests without a media prefix.
    pub no_protocol: bool,
}

impl LauncherOverlay {
    pub fn validate(&self) -> Result<()> {
        ensure!((1..=64).contains(&self.archives.len()), "KiriKiri launcher archive count");
        let mut names = std::collections::BTreeSet::new();
        for archive in &self.archives {
            let path = VfsPath::new(Mount::Asset, archive).map_err(err)?;
            let folded = archive.to_lowercase();
            ensure!(
                path.relative() == archive && !archive.contains(['/', '>'])
                    && folded.ends_with(".xp3") && folded != "data.xp3"
                    && names.insert(folded),
                "invalid or duplicate KiriKiri launcher XP3: {archive}"
            );
        }
        let mut protocols = std::collections::BTreeSet::new();
        for protocol in &self.protocols {
            ensure!(
                matches!(protocol.as_str(), "arc" | "archive" | "psb") && protocols.insert(protocol),
                "unsupported or duplicate KiriKiri launcher protocol: {protocol}"
            );
        }
        Ok(())
    }
}

struct MountedOverlay {
    archives: Vec<OverlayArchive>,
    protocols: Vec<String>,
    no_protocol: bool,
    reported_hit: std::sync::atomic::AtomicBool,
}

struct OverlayArchive {
    source: String,
    point: String,
    index_prefix: String,
}

struct AutoPath {
    directory: String,
    /// Missing physical directories need only the unenumerated archive lookup.
    probe_loose: bool,
}

pub struct Storage {
    files: Arc<dyn ClientStorage>,
    root: String,
    current_uri: String,
    pub(crate) media: Option<crysirror_core::service::MountedMediaCatalog>,
    /// Lowercased path relative to `root` → provider spelling relative to the
    /// asset mount.
    index: BTreeMap<String, String>,
    /// Lowercased auto-path directories, searched after a direct match.
    auto: Vec<AutoPath>,
    /// Ordered container/directory pairs searched for flat names. File probes
    /// still use the current mounted provider and its exact case policy.
    auto_routes: std::sync::OnceLock<Option<Vec<(String, String)>>>,
    // Native TVPGetPlacedPath caches both hits and misses. This bounded index
    // is private to the mounted game and excludes saves and mutable aliases.
    archive_names: std::sync::Mutex<BTreeMap<String, Option<String>>>,
    launcher_overlay: Option<MountedOverlay>,
    /// Registered compound media (`arc://`), owned by the shared resource service.
    pub compound: Option<(String, crysirror_resource::compound::CompoundStorage)>,
    /// Directory inside each routed container that answers hashed names.
    pub hashed_root: Option<String>,
    /// Name-rewriting media registered by linked plug-ins.
    pub(crate) wrappers: WrapperMedia,
}

/// Media that own no files and rewrite a name onto the ordinary search:
/// lzfs.dll's `lzfs` (LZ4-frame passthrough of the same name) and yuzuex.dll's
/// `proxy` (exact lower-cased `ProxyStorageMap` key to a target storage name).
#[derive(Default)]
pub(crate) struct WrapperMedia {
    pub lzfs: bool,
    /// String values of `ProxyStorageMap`, mirrored from the TJS dictionary;
    /// `None` until yuzuex.dll is linked.
    pub proxy: Option<BTreeMap<String, String>>,
    /// Asset paths reached through `lzfs`, read with LZ4 frame passthrough.
    lz4: std::sync::Mutex<std::collections::BTreeSet<String>>,
    /// Nested wrapper resolutions in progress (proxy targets found through a
    /// proxy auto path); bounded so a cyclic map is a miss, not a hang.
    depth: std::sync::atomic::AtomicUsize,
    /// psd.dll's `psd` media; `None` until the plug-in is linked.
    pub psd: Option<crate::psd_plugin::PsdMedia>,
}

impl WrapperMedia {
    fn scheme<'a>(&self, name: &'a str) -> Option<(&'static str, &'a str)> {
        let (scheme, rest) = name.split_once("://")?;
        if self.lzfs && scheme.eq_ignore_ascii_case("lzfs") {
            Some(("lzfs", rest))
        } else if self.proxy.is_some() && scheme.eq_ignore_ascii_case("proxy") {
            Some(("proxy", rest))
        } else {
            None
        }
    }
    /// The ordinary storage name a wrapper URI stands for.
    fn inner(&self, name: &str) -> Option<Result<(&'static str, String)>> {
        let (scheme, rest) = self.scheme(name)?;
        Some((|| {
            let rest = rest.replace('\\', "/");
            ensure!(rest.starts_with("./"), "{scheme} media supports only the root domain `.`");
            if scheme == "lzfs" {
                return Ok((scheme, rest[2..].to_owned()));
            }
            // proxyfs: the media-relative name, ASCII lower-cased, is one exact key.
            let key = rest.to_ascii_lowercase();
            let target = self.proxy.as_ref().and_then(|map| map.get(&key)).cloned()
                .with_context(|| format!("cannot open proxyfile:{rest}"))?;
            ensure!(self.scheme(&target).is_none(), "proxy target cannot be another wrapper media");
            Ok((scheme, target))
        })())
    }
}

impl Storage {
    pub(crate) fn autoload_modules(&self, is_64: bool) -> Result<Vec<String>> {
        let plugin_dir = if is_64 { "plugin64" } else { "plugin" };
        let mut modules: Vec<_> = self.index.keys().filter(|name| {
            let (parent, base) = name.rsplit_once('/').unwrap_or(("", name));
            (parent.is_empty() || parent == "system" || parent == plugin_dir)
                && base.ends_with(".tpm")
        }).cloned().collect();
        ensure!(modules.len() <= 128, "native autoload module limit");
        modules.sort_by(|a,b| a.rsplit('/').next().cmp(&b.rsplit('/').next()).then(a.cmp(b)));
        Ok(modules)
    }

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
            media: None,
            index: BTreeMap::new(),
            auto: Vec::new(),
            auto_routes: Default::default(),
            archive_names: Default::default(),
            launcher_overlay: None,
            compound: None,
            hashed_root: None,
            wrappers: WrapperMedia::default(),
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
        if let Some((scheme, rest)) = self.wrappers.scheme(&name) {
            ensure!(rest.starts_with("./"), "{scheme} media supports only the root domain `.`");
            return Ok(format!("{scheme}://{rest}"));
        }
        let name = if name.as_bytes().get(1) == Some(&b':') {
            let drive = name.chars().next().unwrap().to_ascii_uppercase();
            let volume = self.media.as_ref().and_then(|m| m.volumes().iter().find(|v| v.drive == drive))
                .context("unmounted native drive")?;
            let tail=name.get(2..).context("native drive path")?;
            ensure!(tail.is_empty() || tail.starts_with('/'), "drive-relative native paths unsupported");
            let relative_root=if self.root.is_empty() {volume.root.as_str()}
                else if volume.root == self.root {""}
                else {volume.root.strip_prefix(&format!("{}/",self.root)).context("media outside native root")?};
            let joined=if relative_root.is_empty() {tail.trim_start_matches('/').to_owned()}
                else {format!("{}/{}",relative_root,tail.trim_start_matches('/'))};
            format!("file://./{joined}")
        } else { name };
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
            if let Some(path) = self.routed_member(&candidate.container, &candidate.inner) {
                return Some(path);
            }
        }
        None
    }

    fn routed_member(&self, container: &str, member: &str) -> Option<String> {
        self.search_mounted_member(container, member, [self.hashed_root.as_deref()])
    }

    fn explicit_archive_member(&self, container: &str, member: &str) -> Option<String> {
        // Ordinary entries retain their exact mount path. HXV4 exposes its
        // authenticated logical names below a separate provider root.
        let roots = std::iter::once(None).chain(self.hashed_root.as_deref().map(Some));
        self.search_mounted_member(container, member, roots)
    }

    fn search_mounted_member<'a>(
        &self,
        container: &str,
        member: &str,
        roots: impl IntoIterator<Item = Option<&'a str>>,
    ) -> Option<String> {
        let point = self.container_point(container).ok()??;
        let scoped = |name: &str| {
            if self.root.is_empty() {
                name.to_owned()
            } else {
                format!("{}/{name}", self.root)
            }
        };
        let source = scoped(container);
        let mount = if point.is_empty() {
            self.root.clone()
        } else {
            scoped(&point)
        };
        let belongs_to_mount = |path: &str| {
            VfsPath::new(Mount::Asset, path).ok().is_some_and(|path| {
                self.files.serving_container(&path).is_some_and(|served| {
                    served.0 == source && served.1 == mount
                })
            })
        };
        for root in roots {
            let mut prefix = point.clone();
            if !prefix.is_empty() {
                prefix.push('/');
            }
            if let Some(root) = root {
                prefix.push_str(root);
                prefix.push('/');
            }
            let relative = format!("{prefix}{member}");
            let path = scoped(&relative);
            if belongs_to_mount(&path) {
                return Some(path);
            }
            let folded = relative.to_lowercase();
            if let Some(path) = self.index.get(&folded).filter(|path| belongs_to_mount(path)) {
                return Some(path.clone());
            }
            let folded_member = member.to_lowercase();
            if folded_member != member {
                let path = scoped(&format!("{prefix}{folded_member}"));
                if belongs_to_mount(&path) {
                    return Some(path);
                }
            }
        }
        None
    }
    /// krkr `TVPGetPlacedPath`: the full storage name of `name` as found,
    /// in archive form (`file://./data.xp3>startup.tjs`) when a mounted
    /// container serves it. Hashed-name containers report the logical name.
    pub fn placed_path(&self, name: &str) -> Option<String> {
        let path = self.resolve(name)?;
        // resolve already returns a path in this client's asset namespace.
        let served = VfsPath::new(Mount::Asset, &path)
            .ok()
            .and_then(|p| self.files.serving_container(&p));
        let Some((source, point)) = served else {
            return Some(format!("file://./{path}"));
        };
        let strip_root = |v: &str| -> String {
            if self.root.is_empty() {
                v.to_owned()
            } else {
                v.strip_prefix(&format!("{}/", self.root)).unwrap_or(v).to_owned()
            }
        };
        let source = strip_root(&source);
        let inner = if point.is_empty() {
            path.as_str()
        } else {
            path.strip_prefix(&format!("{point}/")).unwrap_or(&path)
        };
        // Keep the directory selected by the compound/auto-path lookup.
        // Only remove the engine's virtual HXV4 mount component.
        let inner = self.hashed_root.as_deref()
            .and_then(|root| inner.strip_prefix(root)?.strip_prefix('/'))
            .unwrap_or(inner);
        Some(format!("file://./{source}>{inner}"))
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
        if let Some((container, member)) = archive_uri_member(&uri) {
            return self.explicit_archive_member(container, member).is_some();
        }
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
        self.name_from_uri(uri)
    }

    fn name_from_uri(&self, uri: String) -> Result<String> {
        if self.compound_relative(&uri).is_some() || self.wrappers.scheme(&uri).is_some() {
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
        let relative = if self.root.is_empty() || dir.is_empty() {
            format!("{}{}", self.root, dir)
        } else { format!("{}/{dir}", self.root) };
        let probe_loose = self.compound.is_none() || (!self.full_uri(path)?.contains('>')
            && !dir.contains("://")
            && self.files.metadata(Mount::Asset, &relative)
                .map_or(true, |entry| entry.is_some_and(|e| e.is_directory)));
        if let Some(entry) = self.auto.iter_mut().find(|e| e.directory == dir) {
            entry.probe_loose = probe_loose;
        } else {
            self.auto.push(AutoPath { directory: dir, probe_loose });
        }
        self.clear_archive_names();
        Ok(())
    }
    pub fn remove_auto_path(&mut self, path: &str) -> Result<()> {
        if self.absent_archive(path) {
            return Ok(());
        }
        let dir = self.full_name(path)?.trim_end_matches('/').to_lowercase();
        self.auto.retain(|entry| entry.directory != dir);
        self.clear_archive_names();
        Ok(())
    }

    /// Resolve a storage name to an asset-relative path, or `None`.
    pub fn resolve(&self, name: &str) -> Option<String> {
        if let Some(path) = self.launcher_override(name) {
            return Some(path);
        }
        if let Some(media) = &self.wrappers.psd {
            if let Some(rest) = crate::psd_plugin::psd_uri(name) {
                return media.exists(self, &rest).then(|| format!("psd://{rest}"));
            }
        }
        if let Some(inner) = self.wrappers.inner(name) {
            use std::sync::atomic::Ordering::Relaxed;
            let (scheme, inner) = inner.ok()?;
            if self.wrappers.depth.fetch_add(1, Relaxed) >= 4 {
                self.wrappers.depth.fetch_sub(1, Relaxed);
                return None;
            }
            let found = self.resolve(&inner);
            self.wrappers.depth.fetch_sub(1, Relaxed);
            let found = found?;
            if scheme == "lzfs" {
                self.wrappers.lz4.lock().ok()?.insert(found.clone());
            }
            return Some(found);
        }
        if let Ok(Some(path)) = save_path(name) {
            return self
                .files
                .exists(&path)
                .then(|| format!("savedata/{}", path.relative()));
        }
        let uri = self.full_uri(name).ok()?;
        if let Some((container, member)) = archive_uri_member(&uri) {
            return self.explicit_archive_member(container, member);
        }
        let normalized = self.name_from_uri(uri).ok()?;
        if self.compound.is_none() || self.wrappers.proxy.is_some() || normalized.len() > 1024 {
            return self.resolve_asset_name(&normalized);
        }
        if let Some(hit) = self.archive_names.lock().unwrap_or_else(|e| e.into_inner()).get(&normalized) {
            return hit.clone();
        }
        let found = self.resolve_asset_name(&normalized);
        if found.as_ref().is_none_or(|path| path.len() <= 4096) {
            let mut names = self.archive_names.lock().unwrap_or_else(|e| e.into_inner());
            if names.len() >= 2048 {
                names.clear();
            }
            names.insert(normalized, found.clone());
        }
        found
    }

    pub(crate) fn install_launcher_overlay(&mut self, config: &LauncherOverlay) -> Result<()> {
        let mut archives = Vec::with_capacity(config.archives.len());
        for name in &config.archives {
            let source = if self.root.is_empty() {
                name.clone()
            } else {
                format!("{}/{name}", self.root)
            };
            let point = self.files.container_mount(&source).map_err(err)?
                .filter(|point| !point.is_empty() && point != &self.root)
                .with_context(|| format!("KiriKiri launcher XP3 `{name}` is not mounted separately"))?;
            let relative = if self.root.is_empty() {
                point.as_str()
            } else {
                point.strip_prefix(&self.root).and_then(|p| p.strip_prefix('/'))
                    .context("KiriKiri launcher XP3 mount outside the native asset root")?
            };
            let index_prefix = relative.to_lowercase();
            archives.push(OverlayArchive { source, point, index_prefix });
        }
        self.launcher_overlay = Some(MountedOverlay {
            archives,
            protocols: config.protocols.clone(),
            no_protocol: config.no_protocol,
            reported_hit: Default::default(),
        });
        Ok(())
    }

    /// Check the original request before compound hashing or auto paths.
    /// The replacement key is the full member path in the declared archive.
    pub(crate) fn launcher_override(&self, name: &str) -> Option<String> {
        let overlay = self.launcher_overlay.as_ref()?;
        let member = match name.split_once("://") {
            Some((protocol, member)) if overlay.protocols.iter().any(|p| p.eq_ignore_ascii_case(protocol)) => member,
            None if overlay.no_protocol => name,
            _ => return None,
        };
        let native = member.replace('\\', "/");
        if native.ends_with('/') || native.contains('>') {
            return None;
        }
        let member = VfsPath::new(Mount::Asset, &native).ok()?;
        let folded = member.relative().to_lowercase();
        if matches!(folded.split('/').next(), Some("savedata" | "__bootstrap" | "__cxdec")) {
            return None;
        }
        for archive in &overlay.archives {
            let key = format!("{}/{folded}", archive.index_prefix);
            let Some(path) = self.index.get(&key) else { continue };
            let path = VfsPath::new(Mount::Asset, path).ok()?;
            if !self.files.serving_container(&path).is_some_and(|(source, point)| {
                source == archive.source && point == archive.point
            }) {
                continue;
            }
            let trace = std::env::var_os("CRYSIRROR_KRKR_TRACE_STORAGE").is_some();
            if !overlay.reported_hit.swap(true, std::sync::atomic::Ordering::Relaxed) || trace {
                let entry = path.relative().strip_prefix(&archive.point)?.strip_prefix('/')?;
                eprintln!("[KiriKiri launcher override] {name} -> {}>{entry}", archive.source);
            }
            return Some(path.relative().to_owned());
        }
        None
    }

    fn resolve_asset_name(&self, normalized: &str) -> Option<String> {
        if let Some(rel) = self.compound_relative(normalized) {
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
        for entry in self.auto.iter().rev() {
            let dir = &entry.directory;
            if self.wrappers.scheme(dir).is_some() {
                if let Some(found) = self.resolve(&format!("{}/{base}", dir.trim_end_matches('/'))) {
                    return Some(found);
                }
                continue;
            }
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
        let candidate = |name: &str, archive_only: bool| {
            let path = if self.root.is_empty() {
                name.to_owned()
            } else {
                format!("{}/{name}", self.root)
            };
            let checked = VfsPath::new(Mount::Asset, &path).ok()?;
            let exists = if archive_only {
                self.files.serving_container(&checked).is_some()
            } else { self.files.exists(&checked) };
            exists.then_some(path)
        };
        if let Some(path) = candidate(normalized, false) {
            return Some(path);
        }
        if key != normalized && let Some(path) = candidate(&key, false) {
            return Some(path);
        }
        for entry in self.auto.iter().rev() {
            let dir = &entry.directory;
            if dir.contains("://") { continue; }
            if let Some(path) = candidate(&format!("{dir}/{base}"), self.compound.is_some() && !entry.probe_loose) {
                return Some(path);
            }
        }
        self.resolve_auto(base)
    }
    /// True when immutable archive contents serve this resolved asset.
    pub(crate) fn is_mounted_asset(&self, path: &VfsPath) -> bool {
        self.files.serving_container(path).is_some()
    }

    /// Auto-path directories inside compound media.
    fn resolve_auto(&self, base: &str) -> Option<String> {
        let (_, media) = self.compound.as_ref()?;
        // Compound routes match the directory. Their final filename is always
        // the supplied basename, regardless of which source directory won.
        let candidates = || self.auto.iter().rev().filter_map(|entry| {
            let dir = self.compound_relative(&entry.directory)?.ok()?;
            let dir = dir.trim_end_matches('/');
            let lookup = if dir.is_empty() { base.to_owned() } else { format!("{dir}/{base}") };
            Some(media.candidates(".", &lookup))
        }).flatten();
        let routes = self.auto_routes.get_or_init(|| {
            let mut routes = Vec::new();
            let mut seen = std::collections::BTreeSet::new();
            for candidate in candidates() {
                let end = candidate.inner.rfind('/').map_or(0, |slash| slash + 1);
                let route = (candidate.container, candidate.inner[..end].to_owned());
                if seen.insert(route.clone()) {
                    if routes.len() >= crysirror_resource::compound::MAX_ROUTES { return None; }
                    routes.push(route);
                }
            }
            Some(routes)
        });
        match routes {
            Some(routes) => routes.iter().find_map(|(container, prefix)| self.routed_member(container, &format!("{prefix}{base}"))),
            None => candidates().find_map(|candidate| self.routed_member(&candidate.container, &candidate.inner)),
        }
    }

    pub(crate) fn clear_archive_names(&mut self) {
        self.auto_routes.take();
        self.archive_names.get_mut().unwrap_or_else(|e| e.into_inner()).clear();
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
        if let Some(bytes) = self.psd_read(path) {
            let bytes = bytes?;
            ensure!(bytes.len() <= max_bytes, "native storage byte limit");
            return Ok(bytes);
        }
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
        Ok(self.lz4_passthrough(&path, out, max_bytes))
    }
    /// Bytes of an asset reached through `lzfs`: a complete LZ4 frame decoded.
    fn lz4_passthrough(&self, path: &VfsPath, bytes: Vec<u8>, limit: usize) -> Vec<u8> {
        if path.mount() == Mount::Asset
            && crysirror_resource::lz4_frame::has_frame_magic(&bytes)
            && self.wrappers.lz4.lock().is_ok_and(|paths| paths.contains(path.relative()))
            && let Some(decoded) = crysirror_resource::lz4_frame::decode_complete_frame(&bytes, limit)
        {
            return decoded;
        }
        bytes
    }
    /// The first `len` bytes of an asset, streamed (container probing
    /// without reading a whole movie).
    pub fn read_prefix(&self, path: &str, len: usize) -> Result<Vec<u8>> {
        if let Some(bytes) = self.psd_read(path) {
            let mut bytes = bytes?;
            bytes.truncate(len);
            return Ok(bytes);
        }
        let reader = self
            .files
            .open_asset(&VfsPath::new(Mount::Asset, path).map_err(err)?)
            .map_err(err)?;
        let mut out = vec![0; len];
        let mut filled = 0;
        while filled < len {
            let n = reader.read_at(filled as u64, &mut out[filled..]).map_err(err)?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        out.truncate(filled);
        Ok(out)
    }

    /// Layer images of the `psd` media (`psd://<file>/root/…/<layer>.bmp`).
    fn psd_read(&self, path: &str) -> Option<Result<Vec<u8>>> {
        let media = self.wrappers.psd.as_ref()?;
        let rest = crate::psd_plugin::psd_uri(path)?;
        Some(media.open(self, &rest))
    }
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        if let Some(bytes) = self.psd_read(path) {
            return bytes;
        }
        if let Some(path) = save_path(path)? {
            return self.files.read(&path).map_err(err);
        }
        let path = VfsPath::new(Mount::Asset, path).map_err(err)?;
        let bytes = self.files.read(&path).map_err(err).with_context(|| format!("read storage `{}`", path.relative()))?;
        Ok(self.lz4_passthrough(&path, bytes, 256 << 20))
    }

    /// fstat `deleteFile`: only the scoped save mount is writable; a missing
    /// file or an asset path (read-only, like an archive) answers false.
    pub fn delete_save(&self, name: &str) -> Result<bool> {
        let Some(path) = save_path(name)? else { return Ok(false) };
        if !self.files.exists(&path) {
            return Ok(false);
        }
        self.files.remove(&path).map_err(err)?;
        Ok(true)
    }
    /// fstat `moveFile` within the save mount (replaces the destination).
    pub fn move_save(&self, from: &str, to: &str) -> Result<bool> {
        let (Some(src), Some(dst)) = (save_path(from)?, save_path(to)?) else {
            return Ok(false);
        };
        if !self.files.exists(&src) {
            return Ok(false);
        }
        let data = self.files.read(&src).map_err(err)?;
        self.files.write_atomic(&dst, &data).map_err(err)?;
        self.files.remove(&src).map_err(err)?;
        Ok(true)
    }
    /// fstat `copyFile` from any readable storage into the save mount.
    pub fn copy_to_save(&self, from: &str, to: &str) -> Result<bool> {
        let Some(dst) = save_path(to)? else { return Ok(false) };
        let Ok(data) = self.read_named(from) else { return Ok(false) };
        self.files.write_atomic(&dst, &data).map_err(err)?;
        Ok(true)
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
    ensure!(!bytes.starts_with(b"TJS2"),"compiled TJS2 bytecode must be executed, not decoded as text");
    crysirror_resource::text_stream::decode_auto(bytes,64*1024*1024)
}

fn archive_uri_member(uri: &str) -> Option<(&str, &str)> {
    let (container, member) = uri.strip_prefix("file://./")?.split_once('>')?;
    (!member.is_empty() && !member.ends_with('/')).then_some((container, member))
}

fn err(e: crysirror_core::Error) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_archive_uris_find_hashed_members_in_the_named_archive() {
        use crysirror_core::{
            state::ClientId,
            vfs::{MemoryProvider, VfsProvider},
        };
        use crysirror_runtime::vfs::DiskVfs;
        use std::sync::Arc;

        struct Container(&'static str, MemoryProvider);
        impl VfsProvider for Container {
            fn kind(&self) -> &'static str { "synthetic-container" }
            fn container_source(&self) -> Option<&str> { Some(self.0) }
            fn read(&self, path: &str) -> crysirror_core::Result<Vec<u8>> {
                self.1.read(path)
            }
            fn exists(&self, path: &str) -> bool { self.1.exists(path) }
            fn list(&self) -> crysirror_core::Result<Vec<String>> {
                // Authenticated name lookups can succeed without enumeration.
                Ok(Vec::new())
            }
        }
        let assets = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::create_dir(assets.path().join("game")).unwrap();
        let mut vfs = DiskVfs::new(assets.path(), saves.path()).unwrap();
        let client = ClientId(1);
        vfs.register(client, "krkr", "test").unwrap();
        let mut base = MemoryProvider::new();
        base.insert("__hxv4/map_object.asd", b"base".to_vec()).unwrap();
        base.insert("__hxv4/base_only.asd", b"base only".to_vec()).unwrap();
        let mut update = MemoryProvider::new();
        update.insert("__hxv4/map_object.asd", b"update".to_vec()).unwrap();
        update.insert("__hxv4/image/map/detail.asd", b"nested".to_vec()).unwrap();
        update.insert("ordinary.asd", b"ordinary".to_vec()).unwrap();
        vfs.mount_provider(client, "game", Arc::new(Container("game/data.xp3", base))).unwrap();
        vfs.mount_provider(
            client,
            "game/__xp3/update",
            Arc::new(Container("game/update.xp3", update)),
        ).unwrap();
        let mut storage = Storage::new(vfs.storage(client).unwrap(), "game").unwrap();
        storage.hashed_root = Some("__hxv4".to_owned());

        let exact = "file://./update.xp3>Map_Object.asd";
        assert_eq!(storage.read_named(exact).unwrap(), b"update");
        assert!(storage.exists_exact(exact));
        assert_eq!(storage.read_named("file://./data.xp3>map_object.asd").unwrap(), b"base");
        assert_eq!(storage.read_named("file://./update.xp3>ordinary.asd").unwrap(), b"ordinary");

        storage.add_auto_path("update.xp3>__hxv4/image/map/").unwrap();
        let placed = storage.placed_path("detail.asd").unwrap();
        assert_eq!(placed, "file://./update.xp3>image/map/detail.asd");
        assert_eq!(storage.read_named(&placed).unwrap(), b"nested");

        storage.add_auto_path("data.xp3>").unwrap();
        let absent = "file://./update.xp3>base_only.asd";
        assert!(storage.resolve(absent).is_none());
        assert!(!storage.exists_exact(absent));

        storage.set_default_path("file://./update.xp3>").unwrap();
        assert_eq!(storage.read_named("map_object.asd").unwrap(), b"update");
    }

    #[test]
    fn mounted_game_rechecks_cached_hits_and_misses_after_auto_path_changes() {
        use crysirror_core::state::ClientId;
        use crysirror_resource::compound::CompoundStorage;
        use crysirror_runtime::vfs::DiskVfs;

        let assets = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::create_dir(assets.path().join("audio")).unwrap();
        std::fs::write(assets.path().join("audio/select.ogg"), b"fixture").unwrap();
        let mut vfs = DiskVfs::new(assets.path(), saves.path()).unwrap();
        let client = ClientId(1);
        vfs.register(client, "krkr", "test").unwrap();
        let mut storage = Storage::new(vfs.storage(client).unwrap(), "").unwrap();
        storage.compound = Some(("arc".into(), CompoundStorage::new("test", &[]).unwrap()));
        assert_eq!(storage.resolve("select.ogg"), None);
        storage.add_auto_path("audio/").unwrap();
        assert_eq!(storage.resolve("select.ogg").as_deref(), Some("audio/select.ogg"));
        storage.remove_auto_path("audio/").unwrap();
        assert_eq!(storage.resolve("select.ogg"), None);
        storage.add_auto_path("late/").unwrap();
        assert_eq!(storage.resolve("line.txt"), None);
        std::fs::create_dir(assets.path().join("late")).unwrap();
        std::fs::write(assets.path().join("late/line.txt"), b"new").unwrap();
        storage.add_auto_path("late/").unwrap();
        assert_eq!(storage.resolve("line.txt").as_deref(), Some("late/line.txt"));
    }

    #[test]
    fn normalizes_prefixes_and_decodes_boms() {
        assert_eq!(normalize("file://./data\\bg.png"), "data/bg.png");
        assert_eq!(decode_text(&[0xFF, 0xFE, b'a', 0, b'b', 0]).unwrap(), "ab");
        assert_eq!(decode_text(b"\xEF\xBB\xBFok").unwrap(), "ok");
        let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode("吉里吉里");
        assert_eq!(decode_text(&sjis).unwrap(), "吉里吉里");
    }
}
