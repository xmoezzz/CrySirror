// SPDX-License-Identifier: MPL-2.0
// Adapted from siglus_rs RealLive archive at 5f358ef1360b0439e92aed2b749e8f25d8721954.
//! `SEEN.TXT`: the scenario archive.
//!
//! The file starts with a table of 10 000 `(offset, length)` pairs, one per
//! scenario number; a zero offset marks an absent scenario. Loose
//! `SEENnnnn.TXT` files next to the archive override its entries (patches
//! and translations ship them this way).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use anyhow::{Result, anyhow};

use crate::vm::compression::{self, Xor2Key};
use crate::vm::nls::Nls;
use crate::vm::scenario::{Header, Scenario};

const TOC_ENTRIES: usize = 10_000;

#[derive(Debug, Clone)]
enum Source {
    Packed { offset: usize, length: usize },
    Loose(Vec<u8>),
}

/// Where the second-level key came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    NotNeeded,
    Known,
    Derived,
    Missing,
}

#[derive(Debug)]
pub struct Archive {
    regname: String,
    data: Vec<u8>,
    entries: BTreeMap<i32, Source>,
    cache: RefCell<HashMap<i32, Rc<Scenario>>>,
    xor2: Option<Xor2Key>,
    pub key_source: KeySource,
    pub nls: Nls,
}

impl Archive {
    /// Parse capability-read archive bytes. No ambient filesystem is available.
    pub fn from_bytes(data: Vec<u8>, regname: &str, nls: Nls) -> Result<Self> {
        Self::from_parts(data, BTreeMap::new(), regname, nls)
    }

    /// Loose SEEN bytes override packed entries, preserving original precedence.
    pub fn from_parts(
        data: Vec<u8>,
        loose: BTreeMap<i32, Vec<u8>>,
        regname: &str,
        nls: Nls,
    ) -> Result<Self> {
        anyhow::ensure!(
            data.len() <= 128 * 1024 * 1024,
            "RealLive archive byte budget"
        );
        anyhow::ensure!(
            data.is_empty() || data.len() >= TOC_ENTRIES * 8,
            "RealLive archive table is truncated"
        );
        let mut entries = BTreeMap::new();
        if !data.is_empty() {
            for index in 0..TOC_ENTRIES {
                let at = index * 8;
                let offset = u32::from_le_bytes(data[at..at + 4].try_into().expect("4")) as usize;
                let length =
                    u32::from_le_bytes(data[at + 4..at + 8].try_into().expect("4")) as usize;
                if offset != 0 {
                    anyhow::ensure!(
                        offset >= TOC_ENTRIES * 8
                            && offset
                                .checked_add(length)
                                .is_some_and(|end| end <= data.len()),
                        "RealLive scene {index} range exceeds archive"
                    );
                    entries.insert(index as i32, Source::Packed { offset, length });
                }
            }
        }
        for (number, bytes) in loose {
            anyhow::ensure!(
                (0..10_000).contains(&number) && bytes.len() <= 64 * 1024 * 1024,
                "RealLive loose scene byte budget/index"
            );
            entries.insert(number, Source::Loose(bytes));
        }
        anyhow::ensure!(
            !entries.is_empty(),
            "RealLive archive contains no scenarios"
        );
        let mut archive = Self {
            regname: regname.to_owned(),
            data,
            entries,
            cache: RefCell::new(HashMap::new()),
            xor2: None,
            key_source: KeySource::NotNeeded,
            nls,
        };
        archive.select_xor2_key(regname)?;
        Ok(archive)
    }

    pub fn with_loose(mut self, loose: BTreeMap<i32, Vec<u8>>) -> Result<Self> {
        for (number, bytes) in loose {
            anyhow::ensure!(
                (0..10_000).contains(&number) && bytes.len() <= 64 * 1024 * 1024,
                "RealLive loose scene byte budget/index"
            );
            self.entries.insert(number, Source::Loose(bytes));
        }
        self.cache.get_mut().clear();
        // Re-select after overrides, as the original archive constructor does.
        self.xor2 = None;
        self.key_source = KeySource::NotNeeded;
        self.select_xor2_key(&self.regname.clone())?;
        Ok(self)
    }

    fn select_xor2_key(&mut self, regname: &str) -> Result<()> {
        let needs = self.entries.keys().take(8).any(|&number| {
            self.raw(number)
                .ok()
                .and_then(|data| Header::parse(&data).ok())
                .is_some_and(|header| header.uses_xor2)
        });
        if !needs {
            return Ok(());
        }
        if let Some(key) = compression::known_xor2_key(regname) {
            self.xor2 = Some(key);
            self.key_source = KeySource::Known;
            return Ok(());
        }
        let mut codes = Vec::new();
        let mut total = 0usize;
        for &number in self.entries.keys() {
            let code = self.raw(number).ok().and_then(|data| {
                Header::parse(&data)
                    .ok()
                    .and_then(|h| h.decompress(&data).ok())
            });
            if let Some(code) = code {
                total = total
                    .checked_add(code.len())
                    .ok_or_else(|| anyhow!("RealLive XOR2 scan byte budget"))?;
                anyhow::ensure!(
                    total <= 128 * 1024 * 1024,
                    "RealLive XOR2 scan exceeds 128 MiB"
                );
                codes.push(code);
            }
        }
        match compression::derive_xor2_key(&codes) {
            Some(key) => {
                self.xor2 = Some(key);
                self.key_source = KeySource::Derived;
            }
            None => self.key_source = KeySource::Missing,
        }
        Ok(())
    }

    pub fn xor2_key(&self) -> Option<&Xor2Key> {
        self.xor2.as_ref()
    }

    pub fn numbers(&self) -> impl Iterator<Item = i32> + '_ {
        self.entries.keys().copied()
    }

    pub fn contains(&self, number: i32) -> bool {
        self.entries.contains_key(&number)
    }

    pub fn first(&self) -> Option<i32> {
        self.entries.keys().next().copied()
    }

    /// The raw (still compressed) scenario file.
    pub fn raw(&self, number: i32) -> Result<Vec<u8>> {
        match self
            .entries
            .get(&number)
            .ok_or_else(|| anyhow!("reallive: SEEN{number:04} does not exist"))?
        {
            Source::Packed { offset, length } => Ok(self.data[*offset..offset + length].to_vec()),
            Source::Loose(bytes) => Ok(bytes.clone()),
        }
    }

    pub fn scenario(&self, number: i32) -> Result<Rc<Scenario>> {
        if let Some(scenario) = self.cache.borrow().get(&number) {
            return Ok(scenario.clone());
        }
        let data = self.raw(number)?;
        let scenario = Rc::new(Scenario::parse(
            number,
            &data,
            self.xor2.as_ref(),
            self.nls,
        )?);
        self.cache.borrow_mut().insert(number, scenario.clone());
        Ok(scenario)
    }

    /// The encoding named by RLdev metadata in any scenario.
    pub fn probable_encoding(&self) -> Option<Nls> {
        self.entries.keys().find_map(|&number| {
            Header::parse(&self.raw(number).ok()?)
                .ok()?
                .rldev_encoding
                .filter(|nls| *nls != Nls::Sjis)
        })
    }
}

/// Assembles a `SEEN.TXT` archive (tests and tools).
pub fn build(scenarios: &[(i32, Vec<u8>)]) -> Vec<u8> {
    let mut out = vec![0u8; TOC_ENTRIES * 8];
    for (number, data) in scenarios {
        let offset = out.len() as u32;
        let at = *number as usize * 8;
        out[at..at + 4].copy_from_slice(&offset.to_le_bytes());
        out[at + 4..at + 8].copy_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
    }
    out
}
