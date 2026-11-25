// SPDX-License-Identifier: MPL-2.0
//! Dialogue (`vox.bin`, the `say` natives): `vox-info` records name the
//! speech lines of `sound1/speech/<language>.psarc` (`<line>.xvag`).
//!
//! `vox-info` (0x30 bytes): +0x00 id, +0x18 the default speaker, +0x20
//! track count, +0x24 tracks. A track (16 bytes) is `{speaker character,
//! flags, line count, lines}`; its key is a character (`drake`,
//! `soldier-a`, …) or `any`. A line (24 bytes) is `{name, 0, 0, flags, id,
//! flags}`. The PPU resolves the record through the speaker's vox
//! controller (event 0x1300F59C, `sub_81FFC`).
use crate::dc::{Dc, sid};
use anyhow::{Result, ensure};
use std::collections::HashMap;

const TRACK_ANY: u32 = 0x80e0_56d1;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub speaker: u32,
    pub lines: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vox {
    pub speaker: u32,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, Default)]
pub struct VoxTable {
    pub vox: HashMap<u32, Vox>,
}

impl VoxTable {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut dc = Dc::default();
        dc.load(sid("vox"), bytes)?;
        let kind = sid("vox-info");
        let mut vox = HashMap::new();
        let entries = dc.modules[0].entries.clone();
        for (name, at) in entries {
            if dc.mem.type_of(at).ok() != Some(kind) {
                continue;
            }
            let (count, tracks) = (dc.mem.u32(at + 0x20)?, dc.mem.u32(at + 0x24)?);
            ensure!(count <= 256, "vox track count");
            let mut out = Vec::with_capacity(count as usize);
            for t in 0..count {
                let r = tracks + 16 * t;
                let (n, lines) = (dc.mem.u32(r + 8)?, dc.mem.u32(r + 12)?);
                ensure!(n <= 4096, "vox line count");
                let mut names = Vec::with_capacity(n as usize);
                for l in 0..n {
                    let p = dc.mem.u32(lines + 24 * l)?;
                    if p != 0 {
                        let s = dc.mem.cstr(p)?;
                        if !s.is_empty() {
                            names.push(s);
                        }
                    }
                }
                out.push(Track { speaker: dc.mem.u32(r)?, lines: names });
            }
            vox.insert(name, Vox { speaker: dc.mem.u32(at + 0x18)?, tracks: out });
        }
        Ok(Self { vox })
    }

    /// The line `speaker` says for `id`: the speaker's own track, else
    /// `any`, else the first; `pick` selects among its lines (the native
    /// selection rule is not decoded).
    pub fn line(&self, id: u32, speakers: &[u32], pick: u32) -> Option<&str> {
        let v = self.vox.get(&id)?;
        let track = speakers.iter().find_map(|s| v.tracks.iter().find(|t| t.speaker == *s && !t.lines.is_empty()))
            .or_else(|| v.tracks.iter().find(|t| t.speaker == TRACK_ANY && !t.lines.is_empty()))
            .or_else(|| v.tracks.iter().find(|t| !t.lines.is_empty()))?;
        Some(&track.lines[pick as usize % track.lines.len()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A module with one `vox-info` whose `drake` and `any` tracks name
    /// lines.
    fn module() -> Vec<u8> {
        let mut d = vec![0u8; 0x200];
        let w = |d: &mut Vec<u8>, at: usize, v: u32| d[at..at + 4].copy_from_slice(&v.to_be_bytes());
        let mut fix = Vec::new();
        w(&mut d, 0, u32::from_be_bytes(*b"DC00"));
        w(&mut d, 4, 1);
        w(&mut d, 8, 0x200);
        // Root script-file at 0x24 (type at 0x20): one entry.
        w(&mut d, 0x10, 0x24);
        fix.push(0x10);
        w(&mut d, 0x20, crate::dc::TYPE_SCRIPT_FILE);
        w(&mut d, 0x28, 1);
        w(&mut d, 0x2c, 0xabcd);
        w(&mut d, 0x30, 0x44);
        fix.push(0x30);
        // vox-info at 0x44.
        w(&mut d, 0x40, sid("vox-info"));
        w(&mut d, 0x44, 0xabcd);
        w(&mut d, 0x64, 2);
        w(&mut d, 0x68, 0x80);
        fix.push(0x68);
        // Tracks at 0x80: drake (1 line at 0xc0), any (2 lines at 0xe0).
        for (k, (who, n, lines)) in [(sid("drake"), 1, 0xc0), (TRACK_ANY, 2, 0xe0)].into_iter().enumerate() {
            w(&mut d, 0x80 + 16 * k, who);
            w(&mut d, 0x88 + 16 * k, n);
            w(&mut d, 0x8c + 16 * k, lines);
            fix.push(0x8c + 16 * k);
        }
        for (at, text) in [(0xc0, 0x140), (0xe0, 0x160), (0xf8, 0x180)] {
            w(&mut d, at, text);
            fix.push(at);
        }
        for (at, text) in [(0x140, "drk-line-001"), (0x160, "any-line-a"), (0x180, "any-line-b")] {
            d[at..at + text.len()].copy_from_slice(text.as_bytes());
        }
        d.extend((fix.len() as u32).to_be_bytes());
        for f in fix {
            d.extend((f as u32).to_be_bytes());
        }
        d
    }

    #[test]
    fn speaker_track_then_any() {
        let t = VoxTable::parse(&module()).unwrap();
        assert_eq!(t.line(0xabcd, &[sid("drake")], 0), Some("drk-line-001"));
        assert_eq!(t.line(0xabcd, &[sid("elena")], 0), Some("any-line-a"));
        assert_eq!(t.line(0xabcd, &[], 1), Some("any-line-b"));
        assert_eq!(t.line(0x1234, &[], 0), None);
    }
}
