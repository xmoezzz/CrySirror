// SPDX-License-Identifier: MPL-2.0
//! IoStore TOC (versions 3..=8) and chunk payload reader.
use anyhow::{bail, ensure, Context, Result};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

pub const MAGIC: &[u8; 16] = b"-==--==--==--==-";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChunkId {
    pub id: u64,
    pub index: u16,
    pub kind: u8,
}
impl ChunkId {
    fn parse(b: &[u8]) -> Self {
        Self {
            id: u64::from_le_bytes(b[0..8].try_into().unwrap()),
            index: u16::from_le_bytes([b[8], b[9]]),
            kind: b[11],
        }
    }
}
/// EIoChunkType (UE 5.x).
pub fn chunk_kind_name(k: u8) -> &'static str {
    match k {
        0 => "Invalid",
        1 => "ExportBundleData",
        2 => "BulkData",
        3 => "OptionalBulkData",
        4 => "MemoryMappedBulkData",
        5 => "ScriptObjects",
        6 => "ContainerHeader",
        7 => "ExternalFile",
        8 => "ShaderCodeLibrary",
        9 => "ShaderCode",
        10 => "PackageStoreEntry",
        11 => "DerivedData",
        12 => "EditorDerivedData",
        13 => "PackageResource",
        _ => "Unknown",
    }
}

#[derive(Clone, Copy, Debug)]
struct Block {
    offset: u64,
    compressed: u32,
    uncompressed: u32,
    method: u8,
}

pub struct Container {
    pub version: u8,
    pub flags: u8,
    pub block_size: u64,
    pub chunks: Vec<ChunkId>,
    offsets: Vec<(u64, u64)>,
    blocks: Vec<Block>,
    pub methods: Vec<String>,
    /// Package path → TOC entry index.
    pub files: BTreeMap<String, usize>,
    ucas: File,
}

fn be40(b: &[u8]) -> u64 {
    b[..5].iter().fold(0u64, |a, x| (a << 8) | u64::from(*x))
}

impl Container {
    pub fn open(utoc: &Path) -> Result<Self> {
        let b = std::fs::read(utoc).with_context(|| format!("read {}", utoc.display()))?;
        ensure!(b.len() >= 144 && &b[..16] == MAGIC, "not an IoStore TOC");
        let version = b[16];
        ensure!(
            (3..=8).contains(&version),
            "unsupported TOC version {version}"
        );
        let u32at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
        let header = u32at(20);
        let (ec, cbc, cbes, cmc, cmnl, cbs, dis) = (
            u32at(24),
            u32at(28),
            u32at(32),
            u32at(36),
            u32at(40),
            u32at(44),
            u32at(48),
        );
        ensure!(cbes == 12, "unexpected compression block entry size {cbes}");
        let flags = b[80];
        ensure!(flags & 2 == 0, "encrypted containers are not supported");
        let seeds = if version >= 4 { u32at(84) } else { 0 };
        let no_ph = if version >= 5 { u32at(96) } else { 0 };
        let mut o = header;
        let need = |o: usize, n: usize| -> Result<()> {
            ensure!(
                o.checked_add(n).is_some_and(|e| e <= b.len()),
                "TOC truncated"
            );
            Ok(())
        };
        need(o, ec * 22)?;
        let chunks = (0..ec).map(|i| ChunkId::parse(&b[o + i * 12..])).collect();
        o += ec * 12;
        let offsets = (0..ec)
            .map(|i| (be40(&b[o + i * 10..]), be40(&b[o + i * 10 + 5..])))
            .collect();
        o += ec * 10 + seeds * 4 + no_ph * 4;
        need(o, cbc * 12)?;
        let blocks = (0..cbc)
            .map(|i| {
                let e = &b[o + i * 12..o + i * 12 + 12];
                let offset = u64::from_le_bytes([e[0], e[1], e[2], e[3], e[4], 0, 0, 0]);
                Block {
                    offset,
                    compressed: u32::from_le_bytes([e[5], e[6], e[7], 0]),
                    uncompressed: u32::from_le_bytes([e[8], e[9], e[10], 0]),
                    method: e[11],
                }
            })
            .collect();
        o += cbc * 12;
        need(o, cmc * cmnl)?;
        let methods = (0..cmc)
            .map(|i| {
                String::from_utf8_lossy(&b[o + i * cmnl..o + (i + 1) * cmnl])
                    .trim_end_matches('\0')
                    .to_owned()
            })
            .collect();
        o += cmc * cmnl;
        if flags & 4 != 0 {
            let hs = u32at(o);
            o += 4 + hs * 2 + cbc * 20;
        }
        need(o, dis)?;
        let files = if flags & 8 != 0 && dis > 0 {
            parse_directory(&b[o..o + dis])?
        } else {
            BTreeMap::new()
        };
        let ucas = File::open(utoc.with_extension("ucas")).context("open .ucas")?;
        Ok(Self {
            version,
            flags,
            block_size: cbs as u64,
            chunks,
            offsets,
            blocks,
            methods,
            files,
            ucas,
        })
    }

    pub fn find(&self, id: ChunkId) -> Option<usize> {
        self.chunks.iter().position(|c| *c == id)
    }
    pub fn chunk_size(&self, entry: usize) -> u64 {
        self.offsets[entry].1
    }

    /// Decompressed payload of a TOC entry.
    pub fn read(&mut self, entry: usize) -> Result<Vec<u8>> {
        let (offset, len) = *self.offsets.get(entry).context("TOC entry out of range")?;
        ensure!(len <= 1 << 31, "chunk too large");
        let mut out = Vec::with_capacity(len as usize);
        let first = (offset / self.block_size) as usize;
        let last = ((offset + len).max(1) - 1) / self.block_size;
        let mut skip = (offset % self.block_size) as usize;
        let mut extractor = oozextract::Extractor::new();
        for bi in first..=last as usize {
            let blk = *self
                .blocks
                .get(bi)
                .context("compression block out of range")?;
            let mut raw = vec![0u8; blk.compressed as usize];
            self.ucas.seek(SeekFrom::Start(blk.offset))?;
            self.ucas.read_exact(&mut raw)?;
            let data = if blk.method == 0 {
                raw
            } else {
                let name = self
                    .methods
                    .get(blk.method as usize - 1)
                    .map(String::as_str)
                    .unwrap_or("?");
                if !name.eq_ignore_ascii_case("oodle") {
                    bail!("unsupported compression method {name}");
                }
                let mut dst = vec![0u8; blk.uncompressed as usize];
                let n = extractor
                    .read_from_slice(&raw, &mut dst)
                    .map_err(|e| anyhow::anyhow!("oodle block {bi}: {e:?}"))?;
                ensure!(n == dst.len(), "oodle block {bi}: short output");
                dst
            };
            let take = (data.len() - skip).min(len as usize - out.len());
            out.extend_from_slice(&data[skip..skip + take]);
            skip = 0;
        }
        ensure!(out.len() as u64 == len, "chunk length mismatch");
        Ok(out)
    }
}

fn parse_directory(d: &[u8]) -> Result<BTreeMap<String, usize>> {
    let mut p = 0usize;
    let u32r = |p: &mut usize| -> Result<u32> {
        ensure!(*p + 4 <= d.len(), "directory index truncated");
        let v = u32::from_le_bytes(d[*p..*p + 4].try_into().unwrap());
        *p += 4;
        Ok(v)
    };
    let fstr = |p: &mut usize| -> Result<String> {
        ensure!(*p + 4 <= d.len(), "directory index truncated");
        let n = i32::from_le_bytes(d[*p..*p + 4].try_into().unwrap());
        *p += 4;
        if n == 0 {
            return Ok(String::new());
        }
        let s = if n < 0 {
            let len = (-n as usize) * 2;
            ensure!(*p + len <= d.len(), "string truncated");
            let u: Vec<u16> = d[*p..*p + len]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            *p += len;
            String::from_utf16_lossy(&u)
        } else {
            let len = n as usize;
            ensure!(*p + len <= d.len(), "string truncated");
            let s = d[*p..*p + len]
                .iter()
                .map(|c| *c as char)
                .collect::<String>();
            *p += len;
            s
        };
        Ok(s.trim_end_matches('\0').to_owned())
    };
    let mount = fstr(&mut p)?;
    let nd = u32r(&mut p)? as usize;
    let mut dirs = Vec::with_capacity(nd);
    for _ in 0..nd {
        dirs.push((u32r(&mut p)?, u32r(&mut p)?, u32r(&mut p)?, u32r(&mut p)?));
    }
    let nf = u32r(&mut p)? as usize;
    let mut files = Vec::with_capacity(nf);
    for _ in 0..nf {
        files.push((u32r(&mut p)?, u32r(&mut p)?, u32r(&mut p)?));
    }
    let ns = u32r(&mut p)? as usize;
    let mut strings = Vec::with_capacity(ns);
    for _ in 0..ns {
        strings.push(fstr(&mut p)?);
    }
    const NONE: u32 = u32::MAX;
    let mut out = BTreeMap::new();
    let mut stack = vec![(0usize, mount.clone())];
    while let Some((i, path)) = stack.pop() {
        let (name, child, _, mut f) = dirs[i];
        let here = if name != NONE {
            format!("{path}{}/", strings[name as usize])
        } else {
            path
        };
        while f != NONE {
            let (fname, next, user) = files[f as usize];
            out.insert(format!("{here}{}", strings[fname as usize]), user as usize);
            f = next;
        }
        let mut c = child;
        while c != NONE {
            stack.push((c as usize, here.clone()));
            c = dirs[c as usize].2;
        }
    }
    Ok(out)
}
