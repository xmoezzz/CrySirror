//! Policy roots handed over by the host, for targets without directory
//! descriptors: the web page passes the picked game folder as
//! [`ResourceFiles`] (read in place, never copied), the consoles their game
//! directory. User data lives in memory and every change goes to
//! [`UserStore`], which the host persists. Names resolve as
//! in the Unix policy: `bgi-resource/` and `bgi-user/` select a root, a bare
//! name tries the user root and then the resource root, and components match
//! ignoring ASCII case.
use super::{FilePolicy, PolicyMetadata, WriterLock, relative, user_data_alias};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{AssetReader, ClientStorage},
};
use ethornell_archive::ArchiveSource;
use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The picked game folder.
pub trait ResourceFiles: Send + Sync {
    /// Every file below the root: `/`-separated path and byte length.
    fn list(&self) -> io::Result<Vec<(String, u64)>>;
    fn open(&self, path: &str) -> io::Result<Box<dyn ArchiveSource>>;
}

/// Receives every change to user data, so the page can persist it.
pub trait UserStore: Send + Sync {
    /// `None` deletes the file.
    fn persist(&self, path: &str, bytes: Option<&[u8]>) -> io::Result<()>;
    /// A backend must provide an atomic rename; never emulate it by deleting
    /// the source before a possibly failing destination write.
    fn rename(&self, _from: &str, _to: &str) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "atomic user rename unavailable",
        ))
    }
}

// Every session owns its roots; no process-global install or first-wins state.
fn core_io(error: crysirror_core::Error) -> io::Error {
    let kind = match error {
        crysirror_core::Error::NotFound => io::ErrorKind::NotFound,
        crysirror_core::Error::ReadOnly => io::ErrorKind::PermissionDenied,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, error.to_string())
}

struct CapabilityResources {
    storage: Arc<dyn ClientStorage>,
    prefix: String,
}
impl CapabilityResources {
    fn path(&self, relative: &str) -> io::Result<VfsPath> {
        let joined = if self.prefix.is_empty() {
            relative.to_string()
        } else {
            format!("{}/{}", self.prefix, relative)
        };
        VfsPath::new(Mount::Asset, &joined).map_err(core_io)
    }
}

/// Retains one client-scoped reader. Seeking changes only this stream's cursor;
/// the capability itself exposes no host path or shared OS file offset.
struct CapabilityStream {
    reader: Arc<dyn AssetReader>,
    position: u64,
}
impl std::fmt::Debug for CapabilityStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BGIClientAssetStream")
    }
}
impl Read for CapabilityStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let count = self.reader.read_at(self.position, out).map_err(core_io)?;
        if count > out.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "asset reader returned excessive length",
            ));
        }
        self.position = self
            .position
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::other("asset cursor overflow"))?;
        Ok(count)
    }
}
impl std::io::Seek for CapabilityStream {
    fn seek(&mut self, from: std::io::SeekFrom) -> io::Result<u64> {
        let value = match from {
            std::io::SeekFrom::Start(v) => i128::from(v),
            std::io::SeekFrom::Current(v) => i128::from(self.position) + i128::from(v),
            std::io::SeekFrom::End(v) => {
                i128::from(self.reader.len().map_err(core_io)?) + i128::from(v)
            }
        };
        self.position = u64::try_from(value)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid asset seek"))?;
        Ok(self.position)
    }
}
impl ArchiveSource for CapabilityStream {
    fn byte_len(&mut self) -> io::Result<u64> {
        self.reader.len().map_err(core_io)
    }
}

fn walk_capability(
    storage: &dyn ClientStorage,
    mount: Mount,
    prefix: &str,
) -> io::Result<Vec<String>> {
    let mut queue = vec![(String::new(), 0usize)];
    let mut files = Vec::new();
    let mut visited = 0usize;
    while let Some((relative_dir, depth)) = queue.pop() {
        if depth > 64 {
            return Err(io::Error::other(
                "capability directory depth budget exceeded",
            ));
        }
        let directory = [prefix, &relative_dir]
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        for name in storage.list(mount, &directory).map_err(core_io)? {
            visited += 1;
            if visited > 100_000 {
                return Err(io::Error::other("capability file count budget exceeded"));
            }
            if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "non-leaf capability listing",
                ));
            }
            if name.to_ascii_lowercase().starts_with(".ethornell-") {
                continue;
            }
            let relative = if relative_dir.is_empty() {
                name
            } else {
                format!("{relative_dir}/{name}")
            };
            let full = if prefix.is_empty() {
                relative.clone()
            } else {
                format!("{prefix}/{relative}")
            };
            let path = VfsPath::new(mount, &full).map_err(core_io)?;
            let is_file = if mount == Mount::Asset {
                match storage.open_asset(&path) {
                    Ok(_) => true,
                    Err(crysirror_core::Error::Invalid("asset stream must be a file")) => false,
                    Err(error) => {
                        // Providers need not expose directories as streams. A
                        // failed open can denote a directory only if listing it
                        // succeeds and actually has children.
                        let children = storage.list(mount, &full).map_err(|_| core_io(error))?;
                        if children.is_empty() {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "unreadable resource entry",
                            ));
                        }
                        false
                    }
                }
            } else {
                match storage.read(&path) {
                    Ok(_) => true,
                    Err(error) => {
                        let children = storage.list(mount, &full).map_err(|_| core_io(error))?;
                        if children.is_empty() {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "unreadable or unclassifiable user entry",
                            ));
                        }
                        false
                    }
                }
            };
            if is_file {
                files.push(relative);
            } else {
                queue.push((relative, depth + 1));
            }
        }
    }
    files.sort();
    Ok(files)
}
impl ResourceFiles for CapabilityResources {
    fn list(&self) -> io::Result<Vec<(String, u64)>> {
        walk_capability(self.storage.as_ref(), Mount::Asset, &self.prefix)?
            .into_iter()
            .map(|name| {
                let len = self
                    .storage
                    .open_asset(&self.path(&name)?)
                    .map_err(core_io)?
                    .len()
                    .map_err(core_io)?;
                Ok((name, len))
            })
            .collect()
    }
    fn open(&self, path: &str) -> io::Result<Box<dyn ArchiveSource>> {
        let reader = self
            .storage
            .open_asset(&self.path(path)?)
            .map_err(core_io)?;
        Ok(Box::new(CapabilityStream {
            reader,
            position: 0,
        }))
    }
}
struct CapabilityUsers(Arc<dyn ClientStorage>);
impl UserStore for CapabilityUsers {
    fn persist(&self, path: &str, bytes: Option<&[u8]>) -> io::Result<()> {
        let path = VfsPath::new(Mount::Save, path).map_err(core_io)?;
        match bytes {
            Some(bytes) => self.0.write_atomic(&path, bytes),
            None => self.0.remove(&path),
        }
        .map_err(core_io)
    }
}

const USER_FILE_LIMIT: u64 = 64 * 1024 * 1024;

fn fold(path: &str) -> String {
    path.to_ascii_lowercase()
}

/// A tree of files keyed by folded path; directories are implied by files
/// and may also exist on their own (user roots create empty ones).
#[derive(Default)]
struct Tree<F> {
    files: BTreeMap<String, (String, F)>,
    dirs: BTreeMap<String, String>,
}

impl<F> Tree<F> {
    fn insert(&mut self, path: &str, file: F) {
        let mut parent = String::new();
        let parts = path.split('/').collect::<Vec<_>>();
        for part in &parts[..parts.len() - 1] {
            if !parent.is_empty() {
                parent.push('/');
            }
            parent.push_str(part);
            self.dirs
                .entry(fold(&parent))
                .or_insert_with(|| parent.clone());
        }
        self.files.insert(fold(path), (path.to_string(), file));
    }

    fn is_dir(&self, key: &str) -> bool {
        key.is_empty() || self.dirs.contains_key(key)
    }

    /// `(relative name, is_dir)` below `dir`, sorted by folded name as the
    /// Unix walk does; `recursive` includes every depth.
    fn children(&self, dir: &str, recursive: bool) -> Vec<(String, bool)> {
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let strip = |actual: &str| actual.get(prefix.len()..).map(str::to_string);
        let mut out = Vec::new();
        let entries = self
            .dirs
            .iter()
            .map(|(key, actual)| (key, actual, true))
            .chain(
                self.files
                    .iter()
                    .map(|(key, (actual, _))| (key, actual, false)),
            );
        for (key, actual, is_dir) in entries {
            if !key.starts_with(&prefix) || key.len() == prefix.len() {
                continue;
            }
            let rest = &key[prefix.len()..];
            if !recursive && rest.contains('/') {
                continue;
            }
            if let Some(name) = strip(actual) {
                out.push((name, is_dir));
            }
        }
        out.sort_by_key(|(name, _)| fold(name));
        out
    }
}

pub(crate) struct HostedRoots {
    resources: Arc<dyn ResourceFiles>,
    store: Arc<dyn UserStore>,
    resource_tree: Tree<u64>,
    user_tree: Mutex<Tree<Vec<u8>>>,
}

impl std::fmt::Debug for HostedRoots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HostedRoots")
    }
}

#[derive(Clone, Copy)]
enum Root {
    Resource,
    User,
}

/// A checked relative name as a folded key ("" is the root).
fn key_of(name: &str) -> io::Result<String> {
    let path = relative(name).ok_or_else(|| io::Error::from(io::ErrorKind::PermissionDenied))?;
    let parts = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>();
    Ok(fold(&parts.join("/")))
}

fn not_found() -> io::Error {
    io::Error::from(io::ErrorKind::NotFound)
}

impl HostedRoots {
    fn metadata_in(&self, root: Root, key: &str) -> Option<PolicyMetadata> {
        match root {
            Root::Resource => {
                if let Some((_, len)) = self.resource_tree.files.get(key) {
                    return Some(PolicyMetadata {
                        is_dir: false,
                        len: *len,
                        readonly: true,
                    });
                }
                self.resource_tree.is_dir(key).then_some(PolicyMetadata {
                    is_dir: true,
                    len: 0,
                    readonly: true,
                })
            }
            Root::User => {
                let tree = self.user_tree.lock().ok()?;
                if let Some((_, bytes)) = tree.files.get(key) {
                    return Some(PolicyMetadata {
                        is_dir: false,
                        len: bytes.len() as u64,
                        readonly: false,
                    });
                }
                tree.is_dir(key).then_some(PolicyMetadata {
                    is_dir: true,
                    len: 0,
                    readonly: false,
                })
            }
        }
    }

    fn read_in(&self, root: Root, key: &str, limit: u64) -> io::Result<Vec<u8>> {
        let budget = || io::Error::new(io::ErrorKind::InvalidData, "file budget exceeded");
        match root {
            Root::Resource => {
                let (actual, len) = self.resource_tree.files.get(key).ok_or_else(not_found)?;
                if *len > limit {
                    return Err(budget());
                }
                let mut bytes = Vec::new();
                self.resources
                    .open(actual)?
                    .take(limit + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() as u64 > limit {
                    return Err(budget());
                }
                Ok(bytes)
            }
            Root::User => {
                let tree = self
                    .user_tree
                    .lock()
                    .map_err(|_| io::Error::other("user tree"))?;
                let (_, bytes) = tree.files.get(key).ok_or_else(not_found)?;
                if bytes.len() as u64 > limit {
                    return Err(budget());
                }
                Ok(bytes.clone())
            }
        }
    }

    /// The user-root name for a write: never the resource root, and with
    /// its parent directory present (created when `create`).
    fn user_target(&self, name: &str, create: bool) -> io::Result<(String, String)> {
        let normalized = user_data_alias(name);
        if normalized.starts_with("bgi-resource/") {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        let rest = normalized.strip_prefix("bgi-user/").unwrap_or(&normalized);
        let key = key_of(rest)?;
        if key.is_empty() {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let mut tree = self
            .user_tree
            .lock()
            .map_err(|_| io::Error::other("user tree"))?;
        // Keep the spelling of existing components; new ones as given.
        let given = rest.replace('\\', "/");
        let parts = given
            .split('/')
            .filter(|p| !p.is_empty() && *p != ".")
            .collect::<Vec<_>>();
        let leaf_of = |path: &str| path.rsplit('/').next().unwrap_or(path).to_string();
        let mut actual = Vec::new();
        let mut prefix = String::new();
        for (index, part) in parts.iter().enumerate() {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(&fold(part));
            if index + 1 == parts.len() {
                let existing = tree
                    .files
                    .get(&prefix)
                    .map(|(path, _)| path)
                    .or(tree.dirs.get(&prefix));
                actual.push(existing.map_or_else(|| part.to_string(), |path| leaf_of(path)));
            } else if let Some(dir) = tree.dirs.get(&prefix) {
                actual.push(leaf_of(dir));
            } else if tree.files.contains_key(&prefix) {
                return Err(io::Error::from(io::ErrorKind::NotADirectory));
            } else if create {
                actual.push(part.to_string());
                tree.dirs.insert(prefix.clone(), actual.join("/"));
            } else {
                return Err(not_found());
            }
        }
        Ok((key, actual.join("/")))
    }

    fn walk(&self, root: Root, dir: &str, recursive: bool) -> io::Result<Vec<(String, bool)>> {
        match root {
            Root::Resource => {
                if !self.resource_tree.is_dir(dir) {
                    return Err(not_found());
                }
                Ok(self.resource_tree.children(dir, recursive))
            }
            Root::User => {
                let tree = self
                    .user_tree
                    .lock()
                    .map_err(|_| io::Error::other("user tree"))?;
                if !tree.is_dir(dir) {
                    return Err(not_found());
                }
                Ok(tree.children(dir, recursive))
            }
        }
    }
}

impl FilePolicy {
    /// Construct from a single client's capabilities, including every resource
    /// stream and original save operation. No ambient filesystem is consulted.
    pub fn from_client(storage: Arc<dyn ClientStorage>, asset_root: &str) -> io::Result<Self> {
        let prefix = if asset_root.is_empty() {
            String::new()
        } else {
            VfsPath::new(Mount::Asset, asset_root)
                .map_err(core_io)?
                .relative()
                .to_string()
        };
        let resources = Arc::new(CapabilityResources {
            storage: storage.clone(),
            prefix,
        });
        let mut user_files = Vec::new();
        let mut total = 0usize;
        for name in walk_capability(storage.as_ref(), Mount::Save, "")? {
            let bytes = storage
                .read(&VfsPath::new(Mount::Save, &name).map_err(core_io)?)
                .map_err(core_io)?;
            if bytes.len() as u64 > USER_FILE_LIMIT {
                return Err(io::Error::other("initial user file exceeds budget"));
            }
            total = total
                .checked_add(bytes.len())
                .ok_or_else(|| io::Error::other("user files size overflow"))?;
            if total > 256 * 1024 * 1024 {
                return Err(io::Error::other("initial user tree exceeds budget"));
            }
            user_files.push((name, bytes));
        }
        Self::from_roots(resources, Arc::new(CapabilityUsers(storage)), user_files)
    }
    pub fn from_roots(
        resources: Arc<dyn ResourceFiles>,
        store: Arc<dyn UserStore>,
        user_files: Vec<(String, Vec<u8>)>,
    ) -> io::Result<Self> {
        let mut resource_tree = Tree::default();
        for (path, len) in resources.list()? {
            if VfsPath::new(Mount::Asset, &path)
                .map_err(core_io)?
                .relative()
                != path
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "noncanonical resource name",
                ));
            }
            let key = key_of(&path)?;
            if key.is_empty()
                || resource_tree.files.contains_key(&key)
                || resource_tree.dirs.contains_key(&key)
                || key.rsplit_once('/').is_some_and(|(parent, _)| {
                    parent
                        .split('/')
                        .scan(String::new(), |prefix, part| {
                            if !prefix.is_empty() {
                                prefix.push('/');
                            }
                            prefix.push_str(part);
                            Some(prefix.clone())
                        })
                        .any(|ancestor| resource_tree.files.contains_key(&ancestor))
                })
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "ambiguous or empty resource filename",
                ));
            }
            resource_tree.insert(&path, len);
        }
        let mut user_tree = Tree::default();
        let mut total_user_bytes = 0usize;
        for (path, bytes) in user_files {
            if VfsPath::new(Mount::Save, &path)
                .map_err(core_io)?
                .relative()
                != path
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "noncanonical user name",
                ));
            }
            let key = key_of(&path)?;
            total_user_bytes = total_user_bytes
                .checked_add(bytes.len())
                .ok_or_else(|| io::Error::other("user tree size overflow"))?;
            if key.is_empty()
                || user_tree.files.contains_key(&key)
                || user_tree.dirs.contains_key(&key)
                || key.rsplit_once('/').is_some_and(|(parent, _)| {
                    parent
                        .split('/')
                        .scan(String::new(), |prefix, part| {
                            if !prefix.is_empty() {
                                prefix.push('/');
                            }
                            prefix.push_str(part);
                            Some(prefix.clone())
                        })
                        .any(|ancestor| user_tree.files.contains_key(&ancestor))
                })
                || total_user_bytes > 256 * 1024 * 1024
                || bytes.len() as u64 > USER_FILE_LIMIT
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid initial user file",
                ));
            }
            user_tree.insert(&path, bytes);
        }
        Ok(Self {
            resource: Path::new("bgi-resource").to_path_buf(),
            user: Path::new("bgi-user").to_path_buf(),
            hosted: Arc::new(HostedRoots {
                resources,
                store,
                resource_tree,
                user_tree: Mutex::new(user_tree),
            }),
        })
    }
    pub fn lock_writer(&self) -> io::Result<WriterLock> {
        Ok(())
    }
    pub fn validate_resource_tree(&self) -> io::Result<()> {
        Ok(())
    }
    pub fn open_manager(&self) -> ethornell_core::Result<ethornell_archive::ResourceManager> {
        use std::io::{Seek, SeekFrom};
        let mut files = Vec::new();
        for (actual, len) in self.hosted.resource_tree.files.values() {
            if *len < 12 {
                continue;
            }
            let mut source = self.hosted.resources.open(actual)?;
            let mut header = [0u8; 12];
            source.read_exact(&mut header)?;
            if ethornell_archive::detect_archive_format_from_bytes(&header)
                == ethornell_archive::ArchiveFormat::Unknown
            {
                continue;
            }
            source.seek(SeekFrom::Start(0))?;
            files.push((self.resource.join(actual), source));
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        ethornell_archive::ResourceManager::from_authorized_sources_with_limits(
            self.resource.clone(),
            files,
            ethornell_archive::ResourceBudget::android(),
        )
    }
    pub fn read_resource(&self, name: &str, limit: u64) -> io::Result<Vec<u8>> {
        let normalized = name.replace('\\', "/");
        if normalized.starts_with("bgi-user/") {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        let key = key_of(
            normalized
                .strip_prefix("bgi-resource/")
                .unwrap_or(&normalized),
        )?;
        self.hosted.read_in(Root::Resource, &key, limit)
    }
    pub fn metadata(&self, name: &str) -> Option<PolicyMetadata> {
        let normalized = name.replace('\\', "/");
        if let Some(rest) = normalized.strip_prefix("bgi-user/") {
            return self.hosted.metadata_in(Root::User, &key_of(rest).ok()?);
        }
        if let Some(rest) = normalized.strip_prefix("bgi-resource/") {
            return self.hosted.metadata_in(Root::Resource, &key_of(rest).ok()?);
        }
        let key = key_of(&normalized).ok()?;
        self.hosted
            .metadata_in(Root::User, &key)
            .or_else(|| self.hosted.metadata_in(Root::Resource, &key))
    }
    pub fn user_metadata(&self, name: &str) -> Option<PolicyMetadata> {
        self.metadata(&user_data_alias(name))
    }
    pub fn enumerate_user(
        &self,
        pattern: &str,
        recursive: bool,
        max_count: usize,
        directories: bool,
    ) -> io::Result<Vec<String>> {
        self.enumerate(&user_data_alias(pattern), recursive, max_count, directories)
    }
    pub fn read_loose(&self, name: &str) -> io::Result<Vec<u8>> {
        let normalized = name.replace('\\', "/");
        if let Some(rest) = normalized.strip_prefix("bgi-resource/") {
            return self
                .hosted
                .read_in(Root::Resource, &key_of(rest)?, 256 * 1024 * 1024);
        }
        if let Some(rest) = normalized.strip_prefix("bgi-user/") {
            return self
                .hosted
                .read_in(Root::User, &key_of(rest)?, USER_FILE_LIMIT);
        }
        let key = key_of(&normalized)?;
        match self.hosted.read_in(Root::User, &key, USER_FILE_LIMIT) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.hosted.read_in(Root::Resource, &key, 256 * 1024 * 1024)
            }
            other => other,
        }
    }
    pub fn enumerate(
        &self,
        pattern: &str,
        recursive: bool,
        max_count: usize,
        directories: bool,
    ) -> io::Result<Vec<String>> {
        let normalized = pattern.replace('\\', "/");
        let (root, name) = if let Some(name) = normalized.strip_prefix("bgi-resource/") {
            (Root::Resource, name)
        } else {
            (
                Root::User,
                normalized.strip_prefix("bgi-user/").unwrap_or(&normalized),
            )
        };
        let path =
            relative(name).ok_or_else(|| io::Error::from(io::ErrorKind::PermissionDenied))?;
        let parent = path.parent().unwrap_or(Path::new(""));
        let wildcard = path.file_name().and_then(|p| p.to_str()).unwrap_or("*");
        let dir = key_of(&parent.to_string_lossy())?;
        let limit = if max_count == 0 {
            10_000
        } else {
            max_count.clamp(1, 10_000)
        };
        Ok(self
            .hosted
            .walk(root, &dir, recursive)?
            .into_iter()
            .filter(|(name, is_dir)| {
                let leaf = name.rsplit('/').next().unwrap_or(name);
                *is_dir == directories
                    && !fold(leaf).starts_with(".ethornell-")
                    && crate::windows_wildcard_matches(wildcard, leaf)
            })
            .take(limit)
            .map(|(name, _)| name.replace('/', "\\"))
            .collect())
    }
    pub fn create_directory(&self, name: &str) -> io::Result<()> {
        let (key, actual) = self.hosted.user_target(name, false)?;
        let mut tree = self
            .hosted
            .user_tree
            .lock()
            .map_err(|_| io::Error::other("user tree"))?;
        if tree.files.contains_key(&key) || tree.dirs.contains_key(&key) {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        tree.dirs.insert(key, actual);
        Ok(())
    }
    pub fn probe_writable(&self, name: &str) -> io::Result<()> {
        let normalized = user_data_alias(name);
        if name.replace('\\', "/").starts_with("bgi-resource/")
            && normalized.starts_with("bgi-user/")
        {
            // As on Unix: materialize the user-side UserData directory.
            let marker = format!("{}/.probe-parent", normalized.trim_end_matches('/'));
            self.hosted.user_target(&marker, true)?;
        }
        if normalized.starts_with("bgi-resource/") {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        let key = key_of(normalized.strip_prefix("bgi-user/").unwrap_or(&normalized))?;
        let tree = self
            .hosted
            .user_tree
            .lock()
            .map_err(|_| io::Error::other("user tree"))?;
        if tree.is_dir(&key) {
            Ok(())
        } else {
            Err(not_found())
        }
    }
    pub fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        let (from_key, from_actual) = self.hosted.user_target(from, false)?;
        let (to_key, to_actual) = self.hosted.user_target(to, true)?;
        let mut tree = self
            .hosted
            .user_tree
            .lock()
            .map_err(|_| io::Error::other("user tree"))?;
        if !tree.files.contains_key(&from_key) {
            return Err(not_found());
        }
        self.hosted.store.rename(&from_actual, &to_actual)?;
        let (_, bytes) = tree.files.remove(&from_key).ok_or_else(not_found)?;
        tree.files.insert(to_key, (to_actual, bytes));
        Ok(())
    }
    pub fn delete(&self, name: &str) -> io::Result<()> {
        let (key, actual) = self.hosted.user_target(name, false)?;
        let mut tree = self
            .hosted
            .user_tree
            .lock()
            .map_err(|_| io::Error::other("user tree"))?;
        if !tree.files.contains_key(&key) {
            return Err(not_found());
        }
        self.hosted.store.persist(&actual, None)?;
        tree.files.remove(&key).ok_or_else(not_found)?;
        Ok(())
    }
    pub fn read_user(&self, name: &str) -> io::Result<Vec<u8>> {
        let (key, _) = self.hosted.user_target(name, false)?;
        self.hosted.read_in(Root::User, &key, USER_FILE_LIMIT)
    }
    pub fn atomic_write(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() as u64 > USER_FILE_LIMIT {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "save too large"));
        }
        let (key, actual) = self.hosted.user_target(name, true)?;
        let mut tree = self
            .hosted
            .user_tree
            .lock()
            .map_err(|_| io::Error::other("user tree"))?;
        if tree.dirs.contains_key(&key) {
            return Err(io::Error::from(io::ErrorKind::IsADirectory));
        }
        let total = tree
            .files
            .values()
            .try_fold(0usize, |n, (_, v)| n.checked_add(v.len()))
            .ok_or_else(|| io::Error::other("user tree size overflow"))?;
        let previous = tree.files.get(&key).map_or(0, |(_, v)| v.len());
        if total - previous + bytes.len() > 256 * 1024 * 1024 {
            return Err(io::Error::other("user tree byte budget exceeded"));
        }
        self.hosted.store.persist(&actual, Some(bytes))?;
        tree.files.insert(key, (actual, bytes.to_vec()));
        Ok(())
    }
}

#[cfg(test)]
mod crysirror_tests {
    use super::*;
    use std::io::{Cursor, Seek, SeekFrom};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Debug)]
    struct Source(Cursor<Vec<u8>>);
    impl Read for Source {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.0.read(out)
        }
    }
    impl Seek for Source {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            self.0.seek(from)
        }
    }
    impl ArchiveSource for Source {
        fn byte_len(&mut self) -> io::Result<u64> {
            Ok(self.0.get_ref().len() as u64)
        }
    }
    struct Files(Vec<(String, Vec<u8>)>);
    impl ResourceFiles for Files {
        fn list(&self) -> io::Result<Vec<(String, u64)>> {
            Ok(self
                .0
                .iter()
                .map(|(n, b)| (n.clone(), b.len() as u64))
                .collect())
        }
        fn open(&self, path: &str) -> io::Result<Box<dyn ArchiveSource>> {
            let bytes = self
                .0
                .iter()
                .find(|(n, _)| n == path)
                .ok_or_else(not_found)?
                .1
                .clone();
            Ok(Box::new(Source(Cursor::new(bytes))))
        }
    }
    #[derive(Default)]
    struct Store {
        files: Mutex<BTreeMap<String, Vec<u8>>>,
        fail: AtomicBool,
    }
    impl UserStore for Store {
        fn persist(&self, name: &str, bytes: Option<&[u8]>) -> io::Result<()> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(io::Error::other("injected persistence failure"));
            }
            let mut files = self.files.lock().unwrap();
            match bytes {
                Some(bytes) => {
                    files.insert(name.to_string(), bytes.to_vec());
                }
                None => {
                    files.remove(name);
                }
            }
            Ok(())
        }
    }
    fn roots(value: &[u8], store: Arc<Store>) -> FilePolicy {
        FilePolicy::from_roots(
            Arc::new(Files(vec![("story".into(), value.to_vec())])),
            store,
            Vec::new(),
        )
        .unwrap()
    }
    #[test]
    fn sessions_have_independent_capabilities_and_failed_persistence_preserves_old_data() {
        let store = Arc::new(Store::default());
        let a = roots(b"first", store.clone());
        let b = roots(b"second", Arc::new(Store::default()));
        assert_eq!(a.read_resource("story", 100).unwrap(), b"first");
        assert_eq!(b.read_resource("story", 100).unwrap(), b"second");
        a.atomic_write("UserData/slot.sud", b"old").unwrap();
        store.fail.store(true, Ordering::SeqCst);
        assert!(a.atomic_write("UserData/slot.sud", b"new").is_err());
        assert_eq!(a.read_user("UserData/slot.sud").unwrap(), b"old");
        assert!(a.delete("UserData/slot.sud").is_err());
        assert_eq!(a.read_user("UserData/slot.sud").unwrap(), b"old");
        assert_eq!(store.files.lock().unwrap()["UserData/slot.sud"], b"old");
        assert!(a.rename("UserData/slot.sud", "UserData/new.sud").is_err());
        assert_eq!(a.read_user("UserData/slot.sud").unwrap(), b"old");
        assert!(b.read_user("UserData/slot.sud").is_err());
    }
    #[test]
    fn rejects_escape_resource_writes_and_case_ambiguous_roots() {
        let store = Arc::new(Store::default());
        let p = roots(b"asset", store.clone());
        for name in [
            "../escape",
            "/absolute",
            "C:\\escape",
            "bgi-resource/story",
            "bgi-user/.ethornell-session.lock",
        ] {
            assert!(p.atomic_write(name, b"bad").is_err(), "{name}");
        }
        assert!(store.files.lock().unwrap().is_empty());
        let aliases = Arc::new(Files(vec![
            ("Story".into(), vec![1]),
            ("STORY".into(), vec![2]),
        ]));
        assert!(FilePolicy::from_roots(aliases, store.clone(), Vec::new()).is_err());
        let escape = Arc::new(Files(vec![("../secret".into(), vec![1])]));
        assert!(FilePolicy::from_roots(escape, store, Vec::new()).is_err());
    }
    #[test]
    fn native_install_userdata_alias_never_imports_or_overwrites_resource_saves() {
        let store = Arc::new(Store::default());
        let policy = FilePolicy::from_roots(
            Arc::new(Files(vec![(
                "UserData/original.sud".into(),
                b"resource-original".to_vec(),
            )])),
            store.clone(),
            Vec::new(),
        )
        .unwrap();
        assert_eq!(
            policy
                .read_loose("bgi-resource/UserData/original.sud")
                .unwrap(),
            b"resource-original"
        );
        assert!(
            policy
                .read_user("bgi-resource/UserData/original.sud")
                .is_err()
        );
        policy.probe_writable("bgi-resource/UserData").unwrap();
        policy
            .atomic_write("bgi-resource/UserData/original.sud", b"private-slot")
            .unwrap();
        assert_eq!(
            policy.read_user("bgi-user/userdata/ORIGINAL.sud").unwrap(),
            b"private-slot"
        );
        assert_eq!(
            policy
                .read_loose("bgi-resource/UserData/original.sud")
                .unwrap(),
            b"resource-original"
        );
        assert_eq!(
            store.files.lock().unwrap()["UserData/original.sud"],
            b"private-slot"
        );
        assert!(policy.read_user("../UserData/original.sud").is_err());
    }

    struct UnreadableSave;
    impl ClientStorage for UnreadableSave {
        fn read(&self, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
            Err(crysirror_core::Error::Io(
                "injected user read failure".into(),
            ))
        }
        fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
            Err(crysirror_core::Error::ReadOnly)
        }
        fn remove(&self, _: &VfsPath) -> crysirror_core::Result<()> {
            Err(crysirror_core::Error::ReadOnly)
        }
        fn list(&self, mount: Mount, dir: &str) -> crysirror_core::Result<Vec<String>> {
            Ok(if mount == Mount::Save && dir.is_empty() {
                vec!["unreadable.sud".into()]
            } else {
                Vec::new()
            })
        }
    }
    #[test]
    fn unreadable_original_save_is_an_error_instead_of_silently_dropped_initial_state() {
        assert!(FilePolicy::from_client(Arc::new(UnreadableSave), "").is_err());
    }

    struct Sparse {
        reads: AtomicUsize,
    }
    impl AssetReader for Sparse {
        fn len(&self) -> crysirror_core::Result<u64> {
            Ok(3 * 1024 * 1024 * 1024)
        }
        fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            let n = out.len().min((self.len()?.saturating_sub(offset)) as usize);
            for (i, b) in out[..n].iter_mut().enumerate() {
                *b = (offset + i as u64) as u8;
            }
            Ok(n)
        }
    }
    #[test]
    fn sparse_large_archive_reader_seeks_without_materializing_the_pack() {
        let reader = Arc::new(Sparse {
            reads: AtomicUsize::new(0),
        });
        let mut stream = CapabilityStream {
            reader: reader.clone(),
            position: 0,
        };
        assert_eq!(
            stream.seek(SeekFrom::End(-3)).unwrap(),
            3 * 1024 * 1024 * 1024 - 3
        );
        let mut out = [0; 8];
        assert_eq!(stream.read(&mut out).unwrap(), 3);
        assert_eq!(&out[..3], &[253, 254, 255]);
        assert_eq!(stream.read(&mut out).unwrap(), 0);
        assert_eq!(reader.reads.load(Ordering::SeqCst), 2);
        assert!(stream.seek(SeekFrom::Current(i64::MIN)).is_err());
    }
}
