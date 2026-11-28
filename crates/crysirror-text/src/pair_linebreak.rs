// SPDX-License-Identifier: MPL-2.0
//! UAX #14 pair-table line-break data with CJK strictness tailoring.
//!
//! Two editions of a per-codepoint break-class table are shipped as compact
//! run-length data (`data/linebreak-*.lbt`):
//!
//! * [`Edition::Bmp`]: 41 classes over U+0000..U+FFFF, the table Ren'Py
//!   6.18 - 8.4 compiled into `textsupport` (`linebreak.pxi`).
//! * [`Edition::Unicode17`]: 49 classes over U+0000..U+1FFFF, derived from
//!   Unicode 17 `LineBreak.txt` (Ren'Py 8.5+).
//!
//! Each edition has four class tables (western and CJK strict/normal/loose
//! tailorings of CJ/AI/hyphen/iteration/centred/prefix/postfix classes) and
//! a pair table indexed by `old * PITCH + new` whose entries are `^`
//! (prohibited), `%` (indirect), `@`/`#` (combining prohibited/indirect)
//! and `_` (direct). The data are Unicode character properties (Unicode
//! terms of use) arranged by the Ren'Py generator (MIT); this module only
//! answers lookups, so any engine can implement pair-table breaking.
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edition {
    Bmp,
    Unicode17,
}

/// Which class table to consult.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strictness {
    Western,
    CjkStrict,
    CjkNormal,
    CjkLoose,
}

pub struct PairTable {
    names: Vec<String>,
    rules: Vec<u8>,
    tables: [Vec<u8>; 4],
}

fn decode(bytes: &[u8]) -> PairTable {
    let mut p = 4usize;
    assert_eq!(&bytes[..4], b"LBT1", "line-break table magic");
    let n = bytes[p] as usize;
    p += 1;
    let mut names = Vec::with_capacity(n);
    for _ in 0..n {
        let l = bytes[p] as usize;
        p += 1;
        names.push(String::from_utf8_lossy(&bytes[p..p + l]).into_owned());
        p += l;
    }
    let rl = u16::from_le_bytes([bytes[p], bytes[p + 1]]) as usize;
    p += 2;
    let rules = bytes[p..p + rl].to_vec();
    p += rl;
    let count = u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap()) as usize;
    p += 4;
    let mut tables: [Vec<u8>; 4] = Default::default();
    for t in &mut tables {
        let runs = u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap());
        p += 4;
        t.reserve(count);
        for _ in 0..runs {
            let mut len = 0usize;
            let mut shift = 0;
            loop {
                let b = bytes[p];
                p += 1;
                len |= ((b & 0x7f) as usize) << shift;
                shift += 7;
                if b & 0x80 == 0 {
                    break;
                }
            }
            let c = bytes[p];
            p += 1;
            t.extend(std::iter::repeat_n(c, len));
        }
        assert_eq!(t.len(), count, "line-break table length");
    }
    PairTable {
        names,
        rules,
        tables,
    }
}

/// The decoded table of an edition (decoded once per process).
pub fn table(edition: Edition) -> &'static PairTable {
    static BMP: OnceLock<PairTable> = OnceLock::new();
    static U17: OnceLock<PairTable> = OnceLock::new();
    match edition {
        Edition::Bmp => BMP.get_or_init(|| decode(include_bytes!("../data/linebreak-bmp.lbt"))),
        Edition::Unicode17 => {
            U17.get_or_init(|| decode(include_bytes!("../data/linebreak-u17.lbt")))
        }
    }
}

impl PairTable {
    /// Class names in class-number order (`OP` = 0, ...).
    pub fn class_names(&self) -> &[String] {
        &self.names
    }
    pub fn class_id(&self, name: &str) -> Option<u8> {
        self.names.iter().position(|n| n == name).map(|i| i as u8)
    }
    /// Number of codepoints the class tables cover.
    pub fn char_count(&self) -> u32 {
        self.tables[0].len() as u32
    }
    /// Classes below the pitch take part in the pair table.
    pub fn pitch(&self) -> u8 {
        self.class_id("PITCH").expect("PITCH class")
    }
    /// Break class of a codepoint inside the table, None outside it.
    pub fn class(&self, strictness: Strictness, cp: u32) -> Option<u8> {
        let t = match strictness {
            Strictness::Western => &self.tables[0],
            Strictness::CjkStrict => &self.tables[1],
            Strictness::CjkNormal => &self.tables[2],
            Strictness::CjkLoose => &self.tables[3],
        };
        t.get(cp as usize).copied()
    }
    /// Pair rule for `old` followed by `new` (both below the pitch).
    pub fn rule(&self, old: u8, new: u8) -> u8 {
        let p = self.pitch() as usize;
        self.rules
            .get(old as usize * p + new as usize)
            .copied()
            .unwrap_or(b'^')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editions_decode() {
        let b = table(Edition::Bmp);
        assert_eq!(b.char_count(), 65536);
        assert_eq!(b.class_names().len(), 41);
        assert_eq!(b.pitch(), 29);
        let u = table(Edition::Unicode17);
        assert_eq!(u.char_count(), 131072);
        assert_eq!(u.class_id("HH"), Some(40));
        let sp = b.class_id("SP").unwrap();
        assert_eq!(b.class(Strictness::Western, 0x20), Some(sp));
        // ID ID is a direct break, AL AL prohibited.
        let id = b.class_id("ID").unwrap();
        let al = b.class_id("AL").unwrap();
        assert_eq!(b.rule(id, id), b'_');
        assert_eq!(b.rule(al, al), b'%');
        assert_eq!(b.class(Strictness::Western, 0x3042), b.class_id("ID"));
    }
}
