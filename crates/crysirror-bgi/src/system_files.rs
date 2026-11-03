//! Native loose-directory/file policy. Matching itself is an engine capability.
use super::*;
use crysirror_core::service::{ClientStorage, StorageMetadata};
pub(super) fn relative(config: &Config, file: &str) -> Result<String> {
    let normalized = file.replace('\\', "/");
    if normalized.is_empty() || normalized.trim_end_matches('/') == "C:/CrySirror" {
        Ok(String::new())
    } else {
        program_io::relative(config, &normalized)
    }
}
pub(super) fn read_loose(
    storage: &dyn ClientStorage,
    config: &Config,
    file: &str,
) -> Result<Option<Vec<u8>>> {
    let name = relative(config, file)?;
    let Some((mount, info)) = metadata(storage, config, file)? else {
        return Ok(None);
    };
    if info.is_directory {
        return Ok(None);
    }
    ensure!(
        info.size.is_none_or(|n| n <= 64 * 1024 * 1024),
        "BGI file copy byte limit"
    );
    let path = VfsPath::new(
        mount,
        &if mount == Mount::Asset {
            config.asset(&name)?
        } else {
            name
        },
    )?;
    let bytes = storage.read(&path)?;
    ensure!(bytes.len() <= 64 * 1024 * 1024, "BGI file copy byte limit");
    Ok(Some(bytes))
}
pub(super) fn metadata(
    storage: &dyn ClientStorage,
    config: &Config,
    file: &str,
) -> Result<Option<(Mount, StorageMetadata)>> {
    let file = relative(config, file)?;
    let asset = if file.is_empty() {
        config.root.clone()
    } else {
        config.asset(&file)?
    };
    for (mount, path) in [(Mount::Save, file), (Mount::Asset, asset)] {
        match storage.metadata(mount, &path) {
            Ok(Some(m)) => return Ok(Some((mount, m))),
            Ok(None) | Err(Error::NotFound) => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(None)
}
pub(super) fn count_files(
    storage: &dyn ClientStorage,
    config: &Config,
    pattern: &str,
    recursive: bool,
) -> Result<i32> {
    Ok(enumerate(
        storage,
        config,
        pattern,
        recursive && pattern.contains('\\'),
        false,
    )?
    .len()
    .min(i32::MAX as usize) as i32)
}
pub(super) fn enumerate(
    storage: &dyn ClientStorage,
    config: &Config,
    pattern: &str,
    recursive: bool,
    directories: bool,
) -> Result<Vec<String>> {
    let normalized = pattern.replace('\\', "/");
    let (root, mask) = normalized
        .rsplit_once('/')
        .unwrap_or(("", normalized.as_str()));
    ensure!(
        !mask.is_empty() && mask.len() <= 1024,
        "BGI native file pattern"
    );
    let root = relative(config, root)?;
    let mut found = std::collections::BTreeMap::new();
    let mut examined = 0usize;
    for mount in [Mount::Save, Mount::Asset] {
        let mut pending = vec![(root.clone(), String::new(), 0usize)];
        while let Some((dir, prefix, depth)) = pending.pop() {
            ensure!(depth <= 64, "BGI native directory depth");
            let path = if mount == Mount::Asset {
                if dir.is_empty() {
                    config.root.clone()
                } else {
                    config.asset(&dir)?
                }
            } else {
                dir.clone()
            };
            let names = match storage.list(mount, &path) {
                Ok(names) => names,
                Err(Error::NotFound) => continue,
                Err(error) => return Err(error.into()),
            };
            for name in names {
                if dir.is_empty() && name == "__bgi" {
                    continue;
                }
                examined = examined
                    .checked_add(1)
                    .context("BGI directory entry overflow")?;
                ensure!(examined <= 1_000_000, "BGI directory enumeration budget");
                let relative = if dir.is_empty() {
                    name.clone()
                } else {
                    format!("{dir}/{name}")
                };
                let path = if mount == Mount::Asset {
                    config.asset(&relative)?
                } else {
                    relative.clone()
                };
                let Some(info) = storage.metadata(mount, &path)? else {
                    continue;
                };
                let output = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}\\{name}")
                };
                if info.is_directory && recursive {
                    pending.push((relative, output.clone(), depth + 1));
                }
                if info.is_directory == directories
                    && crysirror_resource::dos_glob::matches(mask, &name)
                {
                    // A writable overlay shadows the same Windows path in assets.
                    found.entry(output.to_ascii_lowercase()).or_insert(output);
                }
            }
        }
    }
    Ok(found.into_values().collect())
}
