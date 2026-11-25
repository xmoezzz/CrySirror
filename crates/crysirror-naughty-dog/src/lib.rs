// SPDX-License-Identifier: MPL-2.0
//! Native data from the BCAS20097 01.00 disc build. SATD-nd-retail-runtime.
#![forbid(unsafe_code)]
#[cfg(feature = "runtime")]
pub use adapter::register;
pub use adapter::{Config, ENGINE_ID};
pub mod adapter;
pub mod anim;
pub mod anim_sets;
pub mod anim_state;
pub mod anim_tree;
pub mod characters;
pub mod collision;
pub mod dc;
pub mod game;
pub mod ground;
pub mod native_names;
pub mod script;
pub mod ss;
pub mod task;
pub mod geometry;
pub mod menu;
pub mod mesh;
pub mod nav;
pub mod npc;
pub mod package;
pub mod particle;
pub mod player;
pub mod probes;
pub mod region;
pub mod render_settings;
pub mod rsx;
pub mod scene;
pub mod skeleton;
pub mod sound;
pub mod texture;
pub mod trigger;
pub mod vox;
use anyhow::{Context, Result, bail, ensure};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default, Debug)]
pub struct Level {
    pub id: String,
    pub packages: Vec<String>,
    pub actors: Vec<String>,
    pub sound_banks: Vec<String>,
    pub caches: Vec<String>,
}

/// Read pak22.txt. Preserve the order and multiplicity of each resource list.
/// IDs identify levels independently of display names or paths in a host filesystem.
pub fn levels(text: &str) -> Result<BTreeMap<String, Level>> {
    ensure!(text.len() <= 4 * 1024 * 1024, "ND level manifest size");
    let mut result = BTreeMap::new();
    let mut ids = BTreeSet::new();
    let mut current = None;
    for line in text.lines() {
        ensure!(line.len() <= 1024, "ND level directive length");
        let (directive, value) = line
            .split_once(' ')
            .context("missing level directive argument")?;
        ensure!(
            !value.is_empty()
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                && value != "."
                && value != "..",
            "invalid level resource name"
        );
        if directive == "level-name" {
            ensure!(
                result.len() < 65536 && !result.contains_key(value),
                "duplicate or excessive ND levels"
            );
            result.insert(value.to_owned(), Level::default());
            current = Some(value);
            continue;
        }
        let level = result
            .get_mut(current.context("ND directive before level-name")?)
            .context("missing ND level")?;
        match directive {
            "level-id" => {
                ensure!(
                    level.id.is_empty() && ids.insert(value),
                    "duplicate ND level ID"
                );
                level.id = value.into();
            }
            "package" => level.packages.push(value.into()),
            "sound-bank" => level.sound_banks.push(value.into()),
            "actor" => level.actors.push(value.into()),
            "cache" => level.caches.push(value.into()),
            _ => bail!("unknown ND level directive {directive}"),
        }
    }
    ensure!(
        !result.is_empty() && result.values().all(|s| !s.id.is_empty()),
        "missing ND level ID"
    );
    Ok(result)
}

/// Big-endian ID → UTF-8 string dictionary in text1.psarc. Offsets are
/// relative to the first string byte following the sorted eight-byte records.
pub struct Strings<'a>(Vec<(u32, &'a str)>);

impl<'a> Strings<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        ensure!(bytes.len() <= 8 * 1024 * 1024, "ND string table size");
        let count =
            u32::from_be_bytes(bytes.get(..4).context("short string table")?.try_into()?) as usize;
        ensure!(count <= 100_000, "ND string count");
        let end = 4 + 8 * count;
        let pool = bytes.get(end..).context("short ND string index")?;
        let pool = std::str::from_utf8(pool)?;
        ensure!(pool.ends_with('\0'), "unterminated ND string pool");
        // Cache only referenced offsets. A malicious pool of empty strings
        // must not allocate one index entry per byte.
        let mut resolved = BTreeMap::new();
        let mut entries = Vec::with_capacity(count);
        for row in bytes[4..end].chunks_exact(8) {
            let key = u32::from_be_bytes(row[..4].try_into()?);
            let at = u32::from_be_bytes(row[4..].try_into()?) as usize;
            ensure!(
                entries.last().is_none_or(|&(prev, _)| prev < key),
                "unsorted or duplicate ND string ID"
            );
            ensure!(
                at == 0 || pool.as_bytes().get(at - 1) == Some(&0),
                "invalid ND string boundary"
            );
            let text = match resolved.get(&at) {
                Some(&s) => s,
                None => {
                    let s = pool
                        .get(at..)
                        .and_then(|rest| rest.split_once('\0'))
                        .context("invalid ND string offset")?
                        .0;
                    resolved.insert(at, s);
                    s
                }
            };
            entries.push((key, text));
        }
        Ok(Self(entries))
    }
    pub fn entries(&self) -> &[(u32, &'a str)] {
        &self.0
    }
    pub fn get(&self, id: u32) -> Option<&'a str> {
        self.0
            .binary_search_by_key(&id, |entry| entry.0)
            .ok()
            .map(|i| self.0[i].1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn level_dependencies_preserve_repeated_cache_names() {
        let level = levels("level-name area\nlevel-id id\nactor hero\ncache a\ncache a\n")
            .unwrap()
            .remove("area")
            .unwrap();
        assert_eq!(level.id, "id");
        assert_eq!(level.caches, ["a", "a"]);
        assert!(levels("level-name area\nlevel-id id\npackage ../x").is_err());
        assert!(levels("level-name area\nlevel-id id\nlevel-name b\nlevel-id id").is_err());
        assert!(levels("other x").is_err());
    }
    #[test]
    fn localized_ids_address_exact_string_starts() {
        let mut b = [2u32, 13, 0, 20, 4]
            .into_iter()
            .flat_map(u32::to_be_bytes)
            .collect::<Vec<_>>();
        b.extend_from_slice("abc\0光\0".as_bytes());
        let s = Strings::parse(&b).unwrap();
        assert_eq!(s.get(20), Some("光"));
        assert_eq!(s.get(12), None);
        b[19] = 5;
        assert!(Strings::parse(&b).is_err());
        b[19] = 4;
        b[15] = 13;
        assert!(Strings::parse(&b).is_err());
    }
}
