// SPDX-License-Identifier: MPL-2.0
use crate::{Error, Result};
use alloc::{string::String, vec::Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mount {
    Asset,
    Save,
    Cache,
}

/// Validated, relative, UTF-8 logical path. Not an operating-system path.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct VfsPath {
    mount: Mount,
    relative: String,
}

impl VfsPath {
    pub fn new(mount: Mount, relative: &str) -> Result<Self> {
        if relative.is_empty() || relative.len() > 4096 {
            return Err(Error::Invalid("path length"));
        }
        // Backslashes and drive letters must be handled by the legacy adapter,
        // not by the common filesystem. Never silently accept a host path.
        if relative.starts_with('/')
            || relative.contains('\\')
            || relative.contains(':')
            || relative.chars().any(char::is_control)
        {
            return Err(Error::Invalid("absolute or non-canonical path"));
        }
        let mut parts = Vec::new();
        for part in relative.split('/') {
            match part {
                "" | "." => {}
                ".." => return Err(Error::Invalid("parent traversal")),
                _ => parts.push(part),
            }
        }
        if parts.is_empty() {
            return Err(Error::Invalid("empty path"));
        }
        Ok(Self {
            mount,
            relative: parts.join("/"),
        })
    }
    pub fn mount(&self) -> Mount {
        self.mount
    }
    pub fn relative(&self) -> &str {
        &self.relative
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_only_harmless_components() {
        assert_eq!(
            VfsPath::new(Mount::Asset, "./image//a.png")
                .unwrap()
                .relative(),
            "image/a.png"
        );
    }
    #[test]
    fn rejects_escape_and_host_paths() {
        for p in ["../x", "a/../b", "/x", "C:/x", "a\\b", "", ".", "a\0b"] {
            assert!(VfsPath::new(Mount::Asset, p).is_err(), "{p:?}");
        }
    }
    #[test]
    fn preserves_case() {
        assert_ne!(
            VfsPath::new(Mount::Asset, "A"),
            VfsPath::new(Mount::Asset, "a")
        );
    }
}
