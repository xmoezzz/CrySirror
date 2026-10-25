// SPDX-License-Identifier: MPL-2.0
//! UE 5.5+ Zen package headers: summary, name batch and export map, plus the
//! global ScriptObjects table that names engine classes.
use anyhow::{ensure, Context, Result};
use std::collections::HashMap;

fn u32at(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(o..o + 4).context("truncated")?.try_into().unwrap(),
    ))
}
fn u64at(b: &[u8], o: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        b.get(o..o + 8).context("truncated")?.try_into().unwrap(),
    ))
}

/// A name batch: count, string bytes, hash version, hashes, BE length headers, strings.
pub fn read_name_batch(b: &[u8], mut o: usize) -> Result<(Vec<String>, usize)> {
    let count = u32at(b, o)? as usize;
    let _bytes = u32at(b, o + 4)? as usize;
    o += 8;
    if count == 0 {
        return Ok((Vec::new(), o));
    }
    o += 8 + count * 8; // hash version + hashes
    let headers: Vec<u16> = (0..count)
        .map(|i| {
            Ok(u16::from_be_bytes(
                b.get(o + i * 2..o + i * 2 + 2)
                    .context("names")?
                    .try_into()
                    .unwrap(),
            ))
        })
        .collect::<Result<_>>()?;
    o += count * 2;
    let mut names = Vec::with_capacity(count);
    for h in headers {
        let wide = h & 0x8000 != 0;
        let len = (h & 0x7fff) as usize;
        if wide {
            o += o & 1; // UTF-16 strings are 2-byte aligned
            let s = b.get(o..o + len * 2).context("name")?;
            names.push(String::from_utf16_lossy(
                &s.chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect::<Vec<_>>(),
            ));
            o += len * 2;
        } else {
            names.push(String::from_utf8_lossy(b.get(o..o + len).context("name")?).into_owned());
            o += len;
        }
    }
    Ok((names, o))
}

#[derive(Debug, Clone)]
pub struct Export {
    pub name: String,
    pub serial_offset: u64,
    pub serial_size: u64,
    pub outer: u64,
    pub class: u64,
    pub super_: u64,
    pub template: u64,
    pub public_hash: u64,
    pub flags: u32,
}
#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub header_size: usize,
    pub cooked_header_size: u64,
    pub names: Vec<String>,
    pub imports: Vec<u64>,
    /// Public export hashes referenced by PackageImport indices.
    pub import_hashes: Vec<u64>,
    pub exports: Vec<Export>,
}

/// FPackageObjectIndex: top 2 bits type (0 Export, 1 ScriptImport, 2 PackageImport, 3 Null).
impl Package {
    /// Byte range of an export's serialized data within the package chunk.
    pub fn export_range(&self, e: &Export) -> std::ops::Range<usize> {
        // UE 5.7 zen: CookedSerialOffset is relative to the export data,
        // which starts right after the package header.
        let start = self.header_size + e.serial_offset as usize;
        start..start + e.serial_size as usize
    }
}
pub fn index_kind(i: u64) -> u8 {
    (i >> 62) as u8
}

pub fn parse_package(b: &[u8]) -> Result<Package> {
    let has_versioning = u32at(b, 0)?;
    ensure!(
        has_versioning == 0,
        "versioned (uncooked) packages are not supported"
    );
    let header_size = u32at(b, 4)? as usize;
    let name_index = u32at(b, 8)? as usize;
    let import_hashes = u32at(b, 24)? as usize;
    let import_map = u32at(b, 28)? as usize;
    let export_map = u32at(b, 32)? as usize;
    let export_bundles = u32at(b, 36)? as usize;
    // UE 5.5+: Cell import/export map offsets follow; summary is 60 bytes.
    let (names, _) = read_name_batch(b, 60)?;
    let name = |idx: u64| -> String {
        let i = (idx & 0x3fff_ffff) as usize;
        names.get(i).cloned().unwrap_or_default()
    };
    let import_hashes = (import_hashes..import_map)
        .step_by(8)
        .map(|o| u64at(b, o))
        .collect::<Result<Vec<_>>>()?;
    let imports = (import_map..export_map)
        .step_by(8)
        .map(|o| u64at(b, o))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        (export_bundles - export_map) % 72 == 0,
        "unexpected export map entry size"
    );
    let mut exports = Vec::new();
    for o in (export_map..export_bundles).step_by(72) {
        let mapped = u32at(b, o + 16)? as u64;
        exports.push(Export {
            serial_offset: u64at(b, o)?,
            serial_size: u64at(b, o + 8)?,
            name: name(mapped),
            outer: u64at(b, o + 24)?,
            class: u64at(b, o + 32)?,
            super_: u64at(b, o + 40)?,
            template: u64at(b, o + 48)?,
            public_hash: u64at(b, o + 56)?,
            flags: u32at(b, o + 64)?,
        });
    }
    Ok(Package {
        name: name(name_index as u64),
        header_size,
        cooked_header_size: 0,
        names,
        imports,
        import_hashes,
        exports,
    })
}

/// Global script objects: package object index → "/Script/Module.Class".
pub fn parse_script_objects(b: &[u8]) -> Result<HashMap<u64, String>> {
    let (names, mut o) = read_name_batch(b, 0)?;
    let count = u32at(b, o)? as usize;
    o += 4;
    let mut raw = Vec::with_capacity(count);
    for i in 0..count {
        let e = o + i * 32;
        let n = (u32at(b, e)? & 0x3fff_ffff) as usize;
        raw.push((
            names.get(n).cloned().unwrap_or_default(),
            u64at(b, e + 8)?,
            u64at(b, e + 16)?,
        ));
    }
    let by_index: HashMap<u64, (String, u64)> = raw
        .iter()
        .map(|(n, g, outer)| (*g, (n.clone(), *outer)))
        .collect();
    let mut out = HashMap::new();
    for (g, (n, outer)) in &by_index {
        let mut path = n.clone();
        let mut cur = *outer;
        let mut guard = 0;
        while index_kind(cur) != 3 && guard < 16 {
            let Some((pn, po)) = by_index.get(&cur) else {
                break;
            };
            path = format!("{pn}.{path}");
            cur = *po;
            guard += 1;
        }
        out.insert(*g, path);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn name_batch_decodes_ascii_and_utf16_with_alignment() {
        let mut b = Vec::new();
        b.extend(2u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend(0u64.to_le_bytes());
        b.extend([0u8; 16]); // two hashes
        b.extend(3u16.to_be_bytes());
        b.extend((0x8000u16 | 2).to_be_bytes());
        b.extend(b"Box");
        b.push(0); // align UTF-16
        for c in "体素".encode_utf16() {
            b.extend(c.to_le_bytes());
        }
        let (names, end) = read_name_batch(&b, 0).unwrap();
        assert_eq!(names, vec!["Box".to_owned(), "体素".to_owned()]);
        assert_eq!(end, b.len());
        assert_eq!(index_kind(1 << 62), 1);
    }
}
