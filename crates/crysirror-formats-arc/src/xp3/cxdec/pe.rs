// Portions adapted from xmoezzz/xp3-brute, revision c82cb14302138125640478556723705aa720007b.
// Copyright (c) xp3-brute contributors; modifications CrySirror contributors.
// MIT License: see NOTICE-MIT in this directory.
//! Bounded PE32 section view and embedded `internal module` extraction
//! (upstream `src/legacy_cxdec.rs` PeImage and `src/embedded_pe.rs`).
//! Pure data parsing: nothing is mapped, relocated or executed.
use anyhow::{bail, ensure, Result};
use flate2::read::ZlibDecoder;
use std::io::Read;

pub const SCN_EXECUTE: u32 = 0x2000_0000;
pub const SCN_WRITE: u32 = 0x8000_0000;
const INTERNAL_MODULE_MARKER: &[u8] = b"internal module\0";
const MAX_MARKER_TO_ZLIB: usize = 0x100;
pub const MAX_MODULE_BYTES: usize = 64 * 1024 * 1024;
const MAX_EMBEDDED_MODULES: usize = 32;

#[derive(Clone, Debug)]
pub struct Section {
    pub name: String,
    pub virtual_address: u32,
    pub virtual_size: u32,
    pub raw_offset: u32,
    pub raw_size: u32,
    pub characteristics: u32,
}
impl Section {
    pub fn executable(&self) -> bool {
        self.characteristics & SCN_EXECUTE != 0
    }
    fn span(&self) -> u32 {
        self.virtual_size.max(self.raw_size)
    }
    /// File byte range clamped to `len`.
    pub fn file_range(&self, len: usize) -> (usize, usize) {
        let start = (self.raw_offset as usize).min(len);
        (start, start.saturating_add(self.raw_size as usize).min(len))
    }
}

#[derive(Clone, Debug)]
pub struct PeImage {
    pub machine: u16,
    pub image_base: u32,
    pub size_of_headers: u32,
    pub sections: Vec<Section>,
}
impl PeImage {
    pub fn parse(b: &[u8]) -> Result<Self> {
        ensure!(b.len() >= 0x100 && &b[..2] == b"MZ", "not a PE image");
        let pe = u32_at(b, 0x3c)? as usize;
        ensure!(
            pe.checked_add(24).is_some_and(|e| e <= b.len()) && &b[pe..pe + 4] == b"PE\0\0",
            "invalid PE header"
        );
        let coff = pe + 4;
        let machine = u16_at(b, coff)?;
        let count = u16_at(b, coff + 2)? as usize;
        let opt_size = u16_at(b, coff + 16)? as usize;
        let opt = coff + 20;
        ensure!(
            opt_size >= 64 && opt.checked_add(opt_size).is_some_and(|e| e <= b.len()),
            "truncated PE optional header"
        );
        if u16_at(b, opt)? != 0x10b {
            bail!("CXDEC analysis requires a PE32 module");
        }
        let image_base = u32_at(b, opt + 28)?;
        let size_of_headers = u32_at(b, opt + 60)?;
        let table = opt + opt_size;
        ensure!(
            count <= 96 && table + count * 40 <= b.len(),
            "truncated PE section table"
        );
        let mut sections = Vec::with_capacity(count);
        for i in 0..count {
            let p = table + i * 40;
            let raw = &b[p..p + 8];
            let end = raw.iter().position(|&c| c == 0).unwrap_or(8);
            sections.push(Section {
                name: String::from_utf8_lossy(&raw[..end]).into_owned(),
                virtual_size: u32_at(b, p + 8)?,
                virtual_address: u32_at(b, p + 12)?,
                raw_size: u32_at(b, p + 16)?,
                raw_offset: u32_at(b, p + 20)?,
                characteristics: u32_at(b, p + 36)?,
            });
        }
        Ok(Self {
            machine,
            image_base,
            size_of_headers,
            sections,
        })
    }
    pub fn i386(&self) -> bool {
        self.machine == 0x014c
    }
    pub fn file_offset_to_rva(&self, off: usize) -> Option<u32> {
        let off = u32::try_from(off).ok()?;
        if off < self.size_of_headers {
            return Some(off);
        }
        self.sections.iter().find_map(|s| {
            let end = s.raw_offset.checked_add(s.raw_size)?;
            (off >= s.raw_offset && off < end).then(|| s.virtual_address + (off - s.raw_offset))
        })
    }
    pub fn rva_to_file_offset(&self, rva: u32) -> Option<usize> {
        if rva < self.size_of_headers {
            return Some(rva as usize);
        }
        self.sections.iter().find_map(|s| {
            let end = s.virtual_address.checked_add(s.span())?;
            if rva < s.virtual_address || rva >= end {
                return None;
            }
            let d = rva - s.virtual_address;
            (d < s.raw_size).then(|| (s.raw_offset + d) as usize)
        })
    }
    pub fn va_to_file_offset(&self, va: u32) -> Option<usize> {
        self.rva_to_file_offset(va.checked_sub(self.image_base)?)
    }
    pub fn section_for_rva(&self, rva: u32) -> Option<&Section> {
        self.sections
            .iter()
            .find(|s| rva >= s.virtual_address && rva < s.virtual_address.saturating_add(s.span()))
    }
    pub fn is_executable_rva(&self, rva: u32) -> bool {
        self.sections.iter().any(|s| {
            s.executable()
                && rva >= s.virtual_address
                && rva < s.virtual_address.saturating_add(s.span())
        })
    }
    /// File-backed bytes at an RVA; the whole range must be inside one section.
    pub fn slice_rva<'a>(&self, b: &'a [u8], rva: u32, len: usize) -> Option<&'a [u8]> {
        if rva < self.size_of_headers {
            let end = (rva as usize).checked_add(len)?;
            return (end <= self.size_of_headers as usize && end <= b.len())
                .then(|| &b[rva as usize..end]);
        }
        let s = self.section_for_rva(rva)?;
        let d = rva - s.virtual_address;
        if d.checked_add(u32::try_from(len).ok()?)? > s.raw_size {
            return None;
        }
        let off = (s.raw_offset as usize).checked_add(d as usize)?;
        b.get(off..off.checked_add(len)?)
    }
    pub fn exec_ranges(&self, len: usize) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.sections
            .iter()
            .filter(|s| s.executable())
            .map(move |s| s.file_range(len))
            .filter(|(a, b)| a < b)
    }
}

/// Zlib-compressed PE images stored after an `internal module` marker and
/// mapped manually by the host executable at runtime.
pub fn embedded_modules(bytes: &[u8]) -> Vec<(usize, Vec<u8>)> {
    let mut out = Vec::new();
    let mut search = 0usize;
    while out.len() < MAX_EMBEDDED_MODULES {
        let Some(rel) = find(&bytes[search.min(bytes.len())..], INTERNAL_MODULE_MARKER) else {
            break;
        };
        let marker = search + rel;
        let after = marker + INTERNAL_MODULE_MARKER.len();
        let end = after.saturating_add(MAX_MARKER_TO_ZLIB).min(bytes.len());
        let mut z = after + 8;
        while z + 2 <= end {
            let (cmf, flg) = (bytes[z], bytes[z + 1]);
            if cmf & 0x0f != 8 || (u16::from(cmf) * 256 + u16::from(flg)) % 31 != 0 {
                z += 1;
                continue;
            }
            let usize_at = |o: usize| u32_at(bytes, o).map(|v| v as usize).unwrap_or(0);
            let (plain, packed) = (usize_at(z - 8), usize_at(z - 4));
            if plain == 0
                || plain > MAX_MODULE_BYTES
                || packed < 2
                || z.checked_add(packed).is_none_or(|e| e > bytes.len())
            {
                z += 1;
                continue;
            }
            let mut decoded = Vec::with_capacity(plain);
            let ok = ZlibDecoder::new(&bytes[z..z + packed])
                .take(plain as u64 + 1)
                .read_to_end(&mut decoded)
                .is_ok();
            if ok && decoded.len() == plain && PeImage::parse(&decoded).is_ok() {
                out.push((marker, decoded));
                break;
            }
            z += 1;
        }
        search = after;
    }
    out
}

pub fn u16_at(b: &[u8], o: usize) -> Result<u16> {
    let s = b
        .get(o..o.checked_add(2).unwrap_or(usize::MAX))
        .ok_or_else(|| anyhow::anyhow!("truncated PE field"))?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}
pub fn u32_at(b: &[u8], o: usize) -> Result<u32> {
    let s = b
        .get(o..o.checked_add(4).unwrap_or(usize::MAX))
        .ok_or_else(|| anyhow::anyhow!("truncated PE field"))?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
pub fn u32_opt(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(o..o.checked_add(4)?)?.try_into().ok()?,
    ))
}
pub fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}
pub fn contains(hay: &[u8], needle: &[u8]) -> bool {
    find(hay, needle).is_some()
}
pub fn contains_u32(hay: &[u8], v: u32) -> bool {
    contains(hay, &v.to_le_bytes())
}
