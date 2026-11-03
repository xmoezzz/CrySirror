//! Atomic no-replace rename with capability-resolved directory handles.
use super::*;
pub(super) fn rename(source: &Dir, from: &str, target: &Dir, to: &str) -> Result<()> {
    let old = Path::new(from);
    let new = Path::new(to);
    let old_name = old
        .file_name()
        .ok_or(Error::Invalid("rename source leaf"))?;
    let new_name = new
        .file_name()
        .ok_or(Error::Invalid("rename target leaf"))?;
    let parent = |root: &Dir, path: &Path| -> Result<Dir> {
        match path.parent().filter(|p| !p.as_os_str().is_empty()) {
            Some(path) => root.open_dir(path).map_err(io),
            None => root.try_clone().map_err(io),
        }
    };
    let old_dir = parent(source, old)?;
    let new_dir = parent(target, new)?;
    #[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
    {
        rustix::fs::renameat_with(
            &old_dir,
            old_name,
            &new_dir,
            new_name,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(|error| io(std::io::Error::from(error)))
    }
    #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
    {
        let _ = (old_dir, new_dir, old_name, new_name);
        Err(Error::Unsupported("atomic no-replace rename on this host"))
    }
}
