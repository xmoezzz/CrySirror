// SPDX-License-Identifier: MPL-2.0
//! Insertion-ordered hash table used for dicts, sets and namespaces.
//!
//! The table stores precomputed Python hashes. Key equality may need to call
//! back into Python (`__eq__`), so lookups that can run user code live in
//! `ops` and only use `candidates` here while no borrow is held.

use crate::object::Value;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

#[derive(Default)]
pub struct IdHasher(u64);
impl Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = self.0.rotate_left(8) ^ *b as u64;
        }
    }
    fn write_i64(&mut self, i: i64) {
        self.0 = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

#[derive(Clone)]
enum Slots {
    One(u32),
    Many(Vec<u32>),
}

#[derive(Clone)]
pub struct Entry {
    pub hash: i64,
    pub key: Value,
    pub value: Value,
}

#[derive(Clone, Default)]
pub struct Dict {
    entries: Vec<Option<Entry>>,
    index: HashMap<i64, Slots, BuildHasherDefault<IdHasher>>,
    len: usize,
    /// Incremented on insertion/removal so iterators can detect resizing.
    pub version: u64,
}

impl Dict {
    pub fn new() -> Dict {
        Dict::default()
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// Capacity of the entry vector (indices for iteration).
    pub fn slots(&self) -> usize {
        self.entries.len()
    }
    pub fn entry(&self, idx: usize) -> Option<&Entry> {
        self.entries.get(idx).and_then(|e| e.as_ref())
    }
    pub fn entry_mut(&mut self, idx: usize) -> Option<&mut Entry> {
        self.entries.get_mut(idx).and_then(|e| e.as_mut())
    }
    /// Entry indices whose hash matches.
    pub fn candidates(&self, hash: i64) -> Vec<usize> {
        match self.index.get(&hash) {
            None => Vec::new(),
            Some(Slots::One(i)) => vec![*i as usize],
            Some(Slots::Many(v)) => v.iter().map(|i| *i as usize).collect(),
        }
    }
    /// Finds a key using a native equality predicate.
    pub fn find_with(&self, hash: i64, mut eq: impl FnMut(&Value) -> bool) -> Option<usize> {
        match self.index.get(&hash)? {
            Slots::One(i) => {
                let e = self.entries[*i as usize].as_ref()?;
                eq(&e.key).then_some(*i as usize)
            }
            Slots::Many(v) => v.iter().copied().find(|i| self.entries[*i as usize].as_ref().is_some_and(|e| eq(&e.key))).map(|i| i as usize),
        }
    }
    pub fn insert_new(&mut self, hash: i64, key: Value, value: Value) -> usize {
        let idx = self.entries.len();
        self.entries.push(Some(Entry { hash, key, value }));
        match self.index.get_mut(&hash) {
            None => {
                self.index.insert(hash, Slots::One(idx as u32));
            }
            Some(s) => match s {
                Slots::One(i) => *s = Slots::Many(vec![*i, idx as u32]),
                Slots::Many(v) => v.push(idx as u32),
            },
        }
        self.len += 1;
        self.version += 1;
        idx
    }
    pub fn remove_at(&mut self, idx: usize) -> Option<Entry> {
        let e = self.entries.get_mut(idx)?.take()?;
        if let Some(s) = self.index.get_mut(&e.hash) {
            match s {
                Slots::One(_) => {
                    self.index.remove(&e.hash);
                }
                Slots::Many(v) => {
                    v.retain(|i| *i as usize != idx);
                    if v.len() == 1 {
                        *s = Slots::One(v[0]);
                    }
                }
            }
        }
        self.len -= 1;
        self.version += 1;
        if self.entries.len() > 16 && self.len * 2 < self.entries.len() && self.len == 0 {
            self.entries.clear();
        }
        Some(e)
    }
    /// Compacts tombstones. Invalidates entry indices.
    pub fn compact(&mut self) {
        if self.len == self.entries.len() {
            return;
        }
        let old = std::mem::take(&mut self.entries);
        self.index.clear();
        self.len = 0;
        for e in old.into_iter().flatten() {
            self.insert_new(e.hash, e.key, e.value);
        }
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.index.clear();
        self.len = 0;
        self.version += 1;
    }
    pub fn iter(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().flatten()
    }
    pub fn keys(&self) -> Vec<Value> {
        self.iter().map(|e| e.key.clone()).collect()
    }
    pub fn values(&self) -> Vec<Value> {
        self.iter().map(|e| e.value.clone()).collect()
    }
    pub fn items(&self) -> Vec<(Value, Value)> {
        self.iter().map(|e| (e.key.clone(), e.value.clone())).collect()
    }
    /// Removes and returns the last entry (`popitem`).
    pub fn pop_last(&mut self) -> Option<Entry> {
        let idx = self.entries.iter().rposition(|e| e.is_some())?;
        self.remove_at(idx)
    }
    /// Moves an entry to the end or the start (`OrderedDict.move_to_end`).
    pub fn move_entry(&mut self, idx: usize, last: bool) {
        let Some(e) = self.remove_at(idx) else { return };
        if last {
            self.insert_new(e.hash, e.key, e.value);
        } else {
            let rest: Vec<Entry> = std::mem::take(&mut self.entries).into_iter().flatten().collect();
            self.index.clear();
            self.len = 0;
            self.insert_new(e.hash, e.key, e.value);
            for r in rest {
                self.insert_new(r.hash, r.key, r.value);
            }
        }
    }

    // ---- string-keyed helpers for namespaces (no user __eq__ involved) ----

    pub fn get_str(&self, key: &str) -> Option<&Value> {
        let h = crate::hash::hash_str(key);
        let i = self.find_with(h, |k| k.as_str() == Some(key))?;
        self.entries[i].as_ref().map(|e| &e.value)
    }
    pub fn set_str(&mut self, key: crate::object::Value, value: Value) {
        let s = key.as_str().expect("string key").to_owned();
        let h = crate::hash::hash_str(&s);
        if let Some(i) = self.find_with(h, |k| k.as_str() == Some(&s)) {
            self.entries[i].as_mut().unwrap().value = value;
        } else {
            self.insert_new(h, key, value);
        }
    }
    pub fn del_str(&mut self, key: &str) -> Option<Value> {
        let h = crate::hash::hash_str(key);
        let i = self.find_with(h, |k| k.as_str() == Some(key))?;
        self.remove_at(i).map(|e| e.value)
    }
}
