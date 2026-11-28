// SPDX-License-Identifier: MPL-2.0
//! RLD v1 cmdExHIBIT at resident RVA 0x68b20. The 2011 resident has
//! different entry-name storage and nested services.
use crate::native_numbers::integer;
use crate::registers::{Bank, Registers, Value};
use anyhow::{Result, ensure};

/// FCString's decimal appender (0x9a960) pads before the sign and takes
/// the rightmost characters when the number exceeds the requested width.
pub(super) fn decimal(n: i32, width: usize) -> String {
    let digits = n.to_string();
    if digits.len() >= width {
        digits[digits.len() - width..].to_owned()
    } else {
        format!("{}{}", "0".repeat(width - digits.len()), digits)
    }
}

/// cmdExHIBIT constructs ten FCStrings before splitting its argument tail.
/// Unfilled strings are empty. Its 'last' flag preserves the final remainder.
pub(super) fn args(tail: &[u8], count: i32, remainder: bool) -> Result<[&[u8]; 10]> {
    ensure!(
        count <= 10,
        "ExHibit syscall argument count exceeds native array"
    );
    let mut fields: [&[u8]; 10] = [&[]; 10];
    let count = count.max(0) as usize;
    if remainder {
        for (dst, field) in fields.iter_mut().zip(tail.splitn(count, |b| *b == b',')) {
            *dst = field;
        }
    } else {
        for (dst, field) in fields[..count].iter_mut().zip(tail.split(|b| *b == b',')) {
            *dst = field;
        }
    }
    Ok(fields)
}

#[derive(Debug, PartialEq)]
pub(super) struct Entry {
    slot: i32,
    name: Vec<u8>,
}
/// Local frame variables in liteExec (RVA 0x70c10), outside the scenario
/// array replaced by liteLoadTitle and liteLoadExit.
pub(super) struct Resume {
    pub next: u32,
    pub returns: Vec<(u32, i32)>,
    pub block: i32,
    pub asset_path: crysirror_core::path::VfsPath,
    pub native_path: Vec<u8>,
}

pub(super) fn entry_path(raw: &[u8]) -> Result<(String, crysirror_core::path::VfsPath, Vec<u8>)> {
    use crysirror_core::path::{Mount, VfsPath};
    let name = crysirror_resource::text_stream::decode_shift_jis(raw, 4096)?;
    let stem = name.replace('\\', "/");
    let path = VfsPath::new(Mount::Asset, &format!("rld/{stem}.rld"))?;
    let mut native = br"rld\".to_vec();
    native.extend_from_slice(raw);
    native.extend_from_slice(b".rld");
    Ok((name, path, native))
}

pub(super) struct Entries([Vec<u8>; 5]);
impl Default for Entries {
    fn default() -> Self {
        // liteInit 0x5ef40 initializes all five FCStrings before def.rld.
        Self(std::array::from_fn(|i| {
            if i == 1 {
                b"title".to_vec()
            } else {
                b"exit".to_vec()
            }
        }))
    }
}
impl Entry {
    pub fn parse(count: i32, tail: &[u8]) -> Result<Self> {
        let fields = args(tail, count, true)?;
        Ok(Self {
            slot: integer(fields[0]),
            name: fields[1].to_vec(),
        })
    }
}
impl Entries {
    pub fn set(&mut self, entry: Entry) {
        if let Some(dest) = self.0.get_mut(entry.slot as usize) {
            *dest = entry.name;
        }
    }
    pub fn names(&self) -> &[Vec<u8>; 5] {
        &self.0
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct ExperienceGate {
    bank: Option<Bank>,
    ids: Vec<i32>,
    layer: i32,
    item: i32,
}
impl ExperienceGate {
    pub fn parse(count: i32, tail: &[u8]) -> Result<Self> {
        let f = args(tail, count, false)?;
        let bank = match integer(f[0]) {
            0 => Some(Bank::Cg),
            1 => Some(Bank::Sound),
            2 => Some(Bank::Scene),
            _ => None,
        };
        // Field 2 is converted then discarded. FCString token iteration
        // skips consecutive '.' separators in field 1.
        let ids = f[1]
            .split(|b| *b == b'.')
            .filter(|v| !v.is_empty())
            .map(integer)
            .collect();
        Ok(Self {
            bank,
            ids,
            layer: integer(f[3]),
            item: integer(f[4]),
        })
    }

    pub fn disabled_item(&self, regs: &Registers) -> Result<Option<(u8, u8)>> {
        // cmdExHIBIT 40 passes a direct layer to 0x87690. Unlike call 29,
        // there is no cmdLid2lid character alias lookup.
        if !(0..127).contains(&self.layer) || !(1..256).contains(&self.item) {
            return Ok(None);
        }
        if let Some(bank) = self.bank {
            for &id in &self.ids {
                if regs.read(Value::Seen(bank, id))? != 0 {
                    return Ok(None);
                }
            }
        }
        Ok(Some((self.layer as u8, self.item as u8)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registers::{Change, Limits};
    #[test]
    fn entry_assignment_retains_cp932_and_final_remainder() -> Result<()> {
        let mut entries = Entries::default();
        assert_eq!(entries.names()[0], b"exit");
        assert_eq!(entries.names()[1], b"title");
        entries.set(Entry::parse(2, b"4,\x82\xa0,title")?);
        assert_eq!(entries.names()[4], b"\x82\xa0,title");
        entries.set(Entry::parse(3, b"4,new,discarded")?);
        entries.set(Entry::parse(2, b"5,ignored")?);
        assert_eq!(entries.names()[4], b"new");
        entries.set(Entry::parse(1, b"4")?);
        assert!(entries.names()[4].is_empty());
        Ok(())
    }
    #[test]
    fn gallery_gate_requires_any_positive_counter_in_the_selected_bank() -> Result<()> {
        let mut regs = Registers::new(Some(Limits {
            regs: 99,
            sys: 99,
            cg: 8,
            sound: 8,
            scene: 8,
            strings: 0,
            characters: 0,
            messages: 0,
            locals: 0,
        }))?;
        let mut gate = ExperienceGate::parse(5, b"2,..-1.100.0x4..7,999,42,13")?;
        assert_eq!(gate.disabled_item(&regs)?, Some((42, 13)));
        regs.change(Change::Experience(Bank::Cg, vec![4]))?;
        assert_eq!(gate.disabled_item(&regs)?, Some((42, 13)));
        regs.change(Change::Experience(Bank::Scene, vec![4]))?;
        assert_eq!(gate.disabled_item(&regs)?, None);
        gate = ExperienceGate::parse(5, b"7,4,0,42,13")?;
        assert_eq!(gate.disabled_item(&regs)?, Some((42, 13)));
        for bad in [b"0,1,0,1000,13".as_slice(), b"0,1,0,42,0", b"0,1,0,42,256"] {
            assert_eq!(ExperienceGate::parse(5, bad)?.disabled_item(&regs)?, None);
        }
        Ok(())
    }
}
