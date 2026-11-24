// SPDX-License-Identifier: MPL-2.0
//! DC script modules (`dc1.psarc/*.bin`) and the 32-bit address space the
//! script VM runs in.
//!
//! A module is `"DC00"`, version 1, payload size, 0, a root pointer at 0x10
//! and a trailing fix-up table (count, then payload offsets of pointer
//! words). Every value is preceded by its type `StringId`; the root is a
//! `script-file` `{1, count, [(name, value pointer)]}`. Modules are mapped
//! at distinct bases so script pointers stay valid 32-bit addresses, as on
//! the PPU. Native lookup (`sub_158048`) searches the sorted native table
//! first, then the DC symbol hash table; a later module's definition of a
//! name replaces an earlier one.
use anyhow::{Context, Result, bail, ensure};
use std::collections::HashMap;

/// Naughty Dog `StringId`: CRC-32 MSB-first, polynomial 0x04C11DB7, initial 0.
pub const fn sid(s: &str) -> u32 {
    let b = s.as_bytes();
    let mut c = 0u32;
    let mut i = 0;
    while i < b.len() {
        c ^= (b[i] as u32) << 24;
        let mut k = 0;
        while k < 8 {
            c = if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04C1_1DB7 } else { c << 1 };
            k += 1;
        }
        i += 1;
    }
    c
}

pub const TYPE_SCRIPT_FILE: u32 = sid("script-file");
pub const TYPE_SCRIPT_LAMBDA: u32 = sid("script-lambda");
pub const TYPE_STATE_SCRIPT: u32 = sid("state-script");
pub const TYPE_ARRAY: u32 = sid("array");
pub const TYPE_STRING: u32 = sid("string");

/// First module base; each module gets a 16 MiB window.
const MODULE_BASE: u32 = 0x1000_0000;
const MODULE_WINDOW: u32 = 0x0100_0000;
/// Level package data mapped for script lambdas stored in packages (load
/// regions); each package gets a window rounded up to 16 MiB.
const PACKAGE_BASE: u32 = 0x8000_0000;
/// Script-visible runtime data (locators, strings, variables) allocated by
/// natives.
pub const HEAP_BASE: u32 = 0x0800_0000;
const HEAP_LIMIT: usize = 16 << 20;

struct Region {
    base: u32,
    data: Vec<u8>,
}

/// Script-visible memory: relocated DC modules and the native heap.
pub struct Memory {
    regions: Vec<Region>,
}

impl Default for Memory {
    fn default() -> Self {
        Self { regions: vec![Region { base: HEAP_BASE, data: Vec::new() }] }
    }
}

impl Memory {
    fn region(&self, addr: u32, len: usize) -> Result<(&Region, usize)> {
        for r in &self.regions {
            if addr >= r.base && ((addr - r.base) as usize).checked_add(len).is_some_and(|e| e <= r.data.len()) {
                return Ok((r, (addr - r.base) as usize));
            }
        }
        bail!("script address {addr:#010x} is not mapped")
    }
    pub fn bytes(&self, addr: u32, len: usize) -> Result<&[u8]> {
        let (r, o) = self.region(addr, len)?;
        Ok(&r.data[o..o + len])
    }
    pub fn u32(&self, addr: u32) -> Result<u32> {
        Ok(u32::from_be_bytes(self.bytes(addr, 4)?.try_into()?))
    }
    pub fn f32(&self, addr: u32) -> Result<f32> {
        Ok(f32::from_bits(self.u32(addr)?))
    }
    pub fn write_u32(&mut self, addr: u32, value: u32) -> Result<()> {
        for r in &mut self.regions {
            if addr >= r.base && ((addr - r.base) as usize) + 4 <= r.data.len() {
                let o = (addr - r.base) as usize;
                r.data[o..o + 4].copy_from_slice(&value.to_be_bytes());
                return Ok(());
            }
        }
        bail!("script store to unmapped address {addr:#010x}")
    }
    /// NUL-terminated string at `addr` (at most 1 KiB).
    pub fn cstr(&self, addr: u32) -> Result<String> {
        let (r, o) = self.region(addr, 1)?;
        let tail = &r.data[o..r.data.len().min(o + 1024)];
        let end = tail.iter().position(|b| *b == 0).context("unterminated script string")?;
        Ok(String::from_utf8_lossy(&tail[..end]).into_owned())
    }
    /// The type `StringId` stored before a DC value.
    pub fn type_of(&self, value: u32) -> Result<u32> {
        self.u32(value.checked_sub(4).context("DC value at address 0")?)
    }
    /// Allocate `len` zeroed heap bytes (4-byte aligned).
    pub fn alloc(&mut self, len: usize) -> Result<u32> {
        let heap = &mut self.regions[0];
        let at = heap.data.len().next_multiple_of(4);
        ensure!(at + len <= HEAP_LIMIT, "script heap exhausted");
        heap.data.resize(at + len, 0);
        Ok(HEAP_BASE + at as u32)
    }
    /// A heap copy of `s` with its NUL.
    pub fn alloc_str(&mut self, s: &str) -> Result<u32> {
        let at = self.alloc(s.len() + 1)?;
        let o = (at - HEAP_BASE) as usize;
        self.regions[0].data[o..o + s.len()].copy_from_slice(s.as_bytes());
        Ok(at)
    }
}

/// One loaded module.
#[derive(Clone, Debug)]
pub struct Module {
    pub name: u32,
    pub base: u32,
    pub len: u32,
    /// Top-level `(name, value address)` in file order.
    pub entries: Vec<(u32, u32)>,
}

/// Loaded modules and the DC symbol table.
#[derive(Default)]
pub struct Dc {
    pub mem: Memory,
    pub modules: Vec<Module>,
    pub symbols: HashMap<u32, u32>,
    /// Mapped level packages by name `StringId`, with their base.
    pub packages: HashMap<u32, u32>,
    next_package: u32,
}

impl Dc {
    /// Map a relocated level package (pointer words hold package offsets)
    /// so script lambdas stored in it can run. Returns the package base.
    pub fn map_package(&mut self, name: u32, data: &[u8], pointers: impl Iterator<Item = usize>) -> Result<u32> {
        if let Some(base) = self.packages.get(&name) {
            return Ok(*base);
        }
        let base = PACKAGE_BASE.checked_add(self.next_package).context("package address space")?;
        let window = u32::try_from(data.len().next_multiple_of(MODULE_WINDOW as usize).max(MODULE_WINDOW as usize))?;
        ensure!(base.checked_add(window).is_some_and(|end| end <= 0xF000_0000), "package address space exhausted");
        let mut bytes = data.to_vec();
        for at in pointers {
            ensure!(at % 4 == 0 && at + 4 <= bytes.len(), "package pointer outside data");
            let v = u32::from_be_bytes(bytes[at..at + 4].try_into()?);
            bytes[at..at + 4].copy_from_slice(&(base + v).to_be_bytes());
        }
        self.mem.regions.push(Region { base, data: bytes });
        self.next_package += window;
        self.packages.insert(name, base);
        Ok(base)
    }

    pub fn module(&self, name: u32) -> Option<&Module> {
        self.modules.iter().find(|m| m.name == name)
    }

    /// Relocate and map a module, then define its top-level names. A module
    /// already loaded under `name` is kept.
    pub fn load(&mut self, name: u32, bytes: &[u8]) -> Result<&Module> {
        if let Some(i) = self.modules.iter().position(|m| m.name == name) {
            return Ok(&self.modules[i]);
        }
        ensure!(bytes.len() >= 0x18 && &bytes[..4] == b"DC00", "not a DC module");
        let be = |o: usize| -> Result<u32> { Ok(u32::from_be_bytes(bytes.get(o..o + 4).context("DC module truncated")?.try_into()?)) };
        ensure!(be(4)? == 1, "DC module version {}", be(4)?);
        let size = be(8)? as usize;
        ensure!(size <= bytes.len() && size < MODULE_WINDOW as usize, "DC module size");
        let fixups = be(size)? as usize;
        ensure!(size + 4 + 4 * fixups <= bytes.len(), "DC fix-up table");
        let base = MODULE_BASE + MODULE_WINDOW * u32::try_from(self.modules.len())?;
        ensure!(base < 0xF000_0000, "too many DC modules");
        let mut data = bytes[..size].to_vec();
        for f in 0..fixups {
            let at = be(size + 4 + 4 * f)? as usize;
            ensure!(at % 4 == 0 && at + 4 <= size, "DC fix-up outside module");
            let v = u32::from_be_bytes(data[at..at + 4].try_into()?);
            // An empty array at the end of a module points at its end.
            ensure!((v as usize) <= size, "DC pointer outside module");
            data[at..at + 4].copy_from_slice(&(base + v).to_be_bytes());
        }
        self.mem.regions.push(Region { base, data });
        let root = self.mem.u32(base + 0x10)?;
        ensure!(self.mem.type_of(root)? == TYPE_SCRIPT_FILE, "DC root is not a script-file");
        let count = self.mem.u32(root + 4)?;
        ensure!(count < 65536, "DC entry count");
        let mut entries = Vec::with_capacity(count as usize);
        for i in 0..count {
            let (n, v) = (self.mem.u32(root + 8 + 8 * i)?, self.mem.u32(root + 12 + 8 * i)?);
            entries.push((n, v));
            self.symbols.insert(n, v);
        }
        self.modules.push(Module { name, base, len: size as u32, entries });
        Ok(self.modules.last().expect("pushed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-entry module: `buddy`-style `float` value.
    fn module() -> Vec<u8> {
        let mut b = Vec::new();
        for w in [u32::from_be_bytes(*b"DC00"), 1, 0x30, 0, 0x18, TYPE_SCRIPT_FILE, 1, 1, sid("x"), 0x2c, sid("float"), 1.5f32.to_bits()] {
            b.extend_from_slice(&w.to_be_bytes());
        }
        for w in [2u32, 0x10, 0x24] {
            b.extend_from_slice(&w.to_be_bytes());
        }
        b
    }

    #[test]
    fn known_string_ids() {
        assert_eq!(sid("script-lambda"), 0x9ed4_99e1);
        assert_eq!(sid("go"), 0xc2c0_5f3a);
    }

    #[test]
    fn relocates_and_defines() {
        let mut dc = Dc::default();
        let m = dc.load(sid("m"), &module()).unwrap().clone();
        let v = dc.symbols[&sid("x")];
        assert_eq!(v, m.base + 0x2c);
        assert_eq!(dc.mem.f32(v).unwrap(), 1.5);
        assert_eq!(dc.mem.type_of(v).unwrap(), sid("float"));
        let h = dc.mem.alloc_str("abc").unwrap();
        assert_eq!(dc.mem.cstr(h).unwrap(), "abc");
    }
}
