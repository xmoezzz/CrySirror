// SPDX-License-Identifier: MPL-2.0
//! Level load regions (`REGION_TREE` regions named `level-load…` in the
//! world packages) and the level sets they select (PPU
//! `game/level-mgr.cpp` `sub_BF890`, "[Load Regions] region '%s', task
//! '%s'").
//!
//! `REGION_TREE` is `{data, region pointers…}`. A region holds its name at
//! +0, an `i16` entry count at +0x1c and tag count at +0x1e, tags at +0x20
//! and entries at +0x24. An entry is `{key, type, value}` (12 bytes; type 3
//! is a string). Tag 0xf131b5e1 holds a script lambda whose result selects
//! a scheme N (1 when absent). The handler accepts:
//!
//! * key 0x4f807b48 in every scheme;
//! * `level` and `level-1` for scheme 1, `level-N` for N = 2…8: loaded
//!   and displayed;
//! * `level-no-display`, `level-no-display-1` and `level-no-display-N`:
//!   collected into the second list (loaded without display).
//!
//! The key names were confirmed by their `StringId`s; the name of
//! 0x4f807b48 is not known.
use crate::dc::sid;
use crate::mesh::Loaded;
use anyhow::{Result, ensure};

pub const TAG_SCHEME: u32 = 0xf131_b5e1;
pub const KEY_ALWAYS: u32 = 0x4f80_7b48;

#[derive(Clone, Debug, PartialEq)]
pub struct LoadRegion {
    pub name: String,
    /// Package offset of the scheme lambda (tag 0xf131b5e1).
    pub scheme: Option<usize>,
    pub entries: Vec<(u32, String)>,
}

fn text(p: &Loaded, at: usize) -> String {
    let b = p.data.get(at..).unwrap_or_default();
    let e = b.iter().take(256).position(|c| *c == 0).unwrap_or(0);
    String::from_utf8_lossy(&b[..e]).into_owned()
}

/// The `level-load…` regions of a `REGION_TREE` payload.
pub fn load_regions(p: &Loaded, payload: usize) -> Result<Vec<LoadRegion>> {
    let mut out = Vec::new();
    for i in 1..=65536 {
        let Ok(r) = p.ptr(payload + 4 * i) else { break };
        let Ok(name) = p.ptr(r).map(|a| text(p, a)) else { continue };
        if !name.starts_with("level-load") {
            continue;
        }
        let (count, tags) = (p.u16(r + 0x1c)? as usize, p.u16(r + 0x1e)? as usize);
        ensure!(count <= 4096 && tags <= 256, "ND region record counts");
        let mut scheme = None;
        if let Ok(t) = p.ptr(r + 0x20) {
            for k in 0..tags {
                if p.u32(t + 12 * k)? == TAG_SCHEME {
                    scheme = p.ptr(t + 12 * k + 4).and_then(|holder| p.ptr(holder)).ok();
                }
            }
        }
        let mut entries = Vec::new();
        if let Ok(e) = p.ptr(r + 0x24) {
            for k in 0..count {
                let at = e + 12 * k;
                if p.u32(at + 4)? != 3 {
                    continue;
                }
                if let Ok(v) = p.ptr(at + 8) {
                    entries.push((p.u32(at)?, text(p, v)));
                }
            }
        }
        out.push(LoadRegion { name, scheme, entries });
    }
    Ok(out)
}

/// Levels of scheme `n`: `(displayed, loaded without display)`.
pub fn levels(region: &LoadRegion, n: u32) -> (Vec<String>, Vec<String>) {
    let display: Vec<u32> = if n == 1 { vec![sid("level"), sid("level-1")] } else { vec![sid(&format!("level-{n}"))] };
    let hidden: Vec<u32> = if n == 1 { vec![sid("level-no-display"), sid("level-no-display-1")] } else { vec![sid(&format!("level-no-display-{n}"))] };
    let (mut shown, mut loaded) = (Vec::new(), Vec::new());
    for (key, level) in &region.entries {
        let list = if *key == KEY_ALWAYS || display.contains(key) {
            &mut shown
        } else if hidden.contains(key) {
            &mut loaded
        } else {
            continue;
        };
        if !list.contains(level) {
            list.push(level.clone());
        }
    }
    (shown, loaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_ids() {
        assert_eq!(sid("level-2"), 0xd248_8ba9);
        assert_eq!(sid("level-no-display-2"), 0xba37_565f);
        assert_eq!(sid("level-no-display"), 0x3729_e786);
    }

    #[test]
    fn scheme_selection() {
        let r = LoadRegion {
            name: "level-load-x".into(),
            scheme: None,
            entries: vec![(sid("level-1"), "a".into()), (sid("level-2"), "b".into()), (sid("level-no-display-2"), "c".into()), (KEY_ALWAYS, "d".into())],
        };
        assert_eq!(levels(&r, 2), (vec!["b".to_owned(), "d".into()], vec!["c".to_owned()]));
        assert_eq!(levels(&r, 1).0, vec!["a".to_owned(), "d".into()]);
    }
}
