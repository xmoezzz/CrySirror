// SPDX-License-Identifier: MPL-2.0
//! Resource directory of big-endian PAK version 0xa79. The global page table
//! uses file offsets. Each page's item directory uses page-relative offsets.
use anyhow::{Context, Result, ensure};
use crysirror_core::service::AssetReader;
use crysirror_resource::read_exact_at;

#[derive(Debug)]
pub struct Item {
    pub name: String,
    pub kind: String,
    pub page: usize,
    /// Start of the native payload, following the 32-byte item header.
    pub payload_offset: u64,
}

#[derive(Debug)]
pub struct Directory {
    pub page_count: usize,
    /// Start of VRAM data: file length minus the header's data size.
    pub texture_base: u64,
    pub items: Vec<Item>,
}

fn word(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
}

fn text_at(page: &[u8], pointer: &[u8]) -> Result<String> {
    let at = usize::try_from(u64::from_be_bytes(pointer.try_into()?))?;
    let bytes = page.get(at..).context("ND string outside page")?;
    let end = bytes
        .iter()
        .take(1024)
        .position(|b| *b == 0)
        .context("unterminated ND item name")?;
    Ok(std::str::from_utf8(&bytes[..end])?.into())
}

impl Directory {
    /// Read one page at a time. Item offsets remain data and are never treated
    /// as host addresses. Shaders, geometry and scene-controller state require
    /// further profile-specific interpretation (SATD-nd-retail-runtime).
    pub fn read(source: &dyn AssetReader) -> Result<Self> {
        let mut h = [0; 44];
        read_exact_at(source, 0, &mut h)?;
        ensure!(word(&h, 0) == 0xa79, "unsupported ND package version");
        let header_size = u64::from(word(&h, 4));
        let count = word(&h, 16) as usize;
        let table_at = u64::from(word(&h, 20));
        let len = source.len()?;
        let texture_base = len.checked_sub(u64::from(word(&h, 32))).context("ND VRAM data size")?;
        ensure!(
            count > 0
                && count <= 4096
                && table_at >= 44
                && table_at + count as u64 * 12 <= header_size
                && header_size <= len,
            "invalid ND page table"
        );
        let mut table = vec![0; count * 12];
        read_exact_at(source, table_at, &mut table)?;
        let mut previous = header_size;
        let mut items = Vec::new();
        let mut string_bytes = 0;
        for (page, row) in table.chunks_exact(12).enumerate() {
            let start = u64::from(word(row, 0));
            let size = word(row, 4) as usize;
            ensure!(
                (16..=512 * 1024).contains(&size)
                    && start >= previous
                    && start + size as u64 <= len,
                "ND package page bounds"
            );
            previous = start + size as u64;
            let mut bytes = vec![0; size];
            read_exact_at(source, start, &mut bytes)?;
            let n = word(&bytes, 12) as usize;
            ensure!(
                n <= (size - 16) / 16 && items.len() + n <= 100_000,
                "ND page item count"
            );
            for row in bytes[16..16 + n * 16].chunks_exact(16) {
                let at = word(row, 8) as usize;
                let header = bytes
                    .get(at..)
                    .and_then(|s| s.get(..32))
                    .context("ND item outside page")?;
                ensure!(
                    header[..8] == row[..8] && word(row, 12) == 0x12345,
                    "invalid ND page item link"
                );
                let name = text_at(&bytes, &header[..8])?;
                let kind = text_at(&bytes, &header[8..16])?;
                string_bytes += name.len() + kind.len();
                ensure!(
                    !kind.is_empty() && string_bytes <= 8 * 1024 * 1024,
                    "invalid ND item names"
                );
                items.push(Item {
                    name,
                    kind,
                    page,
                    payload_offset: start + at as u64 + 32,
                });
            }
        }
        ensure!(previous <= texture_base, "ND VRAM data overlaps pages");
        Ok(Self {
            page_count: count,
            texture_base,
            items,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::service::MemoryAssetReader;
    fn words(dst: &mut [u8], at: usize, vals: &[u32]) {
        for (chunk, word) in dst[at..].chunks_exact_mut(4).zip(vals) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
    }
    #[test]
    fn names_are_page_relative_and_links_must_match() {
        let mut b = vec![0; 256];
        words(&mut b, 0, &[0xa79, 64, 0, 0, 1, 44]);
        words(&mut b, 44, &[64, 192, 0]);
        let page = &mut b[64..];
        words(page, 0, &[0, 192, 0, 1]);
        words(page, 16, &[0, 64, 32, 0x12345]);
        words(page, 32, &[0, 64, 0, 70]);
        page[64..70].copy_from_slice(b"stone\0");
        page[70..76].copy_from_slice(b"MODEL\0");
        let dir = Directory::read(&MemoryAssetReader(b.clone().into())).unwrap();
        assert_eq!((dir.page_count, dir.items.len()), (1, 1));
        assert_eq!(dir.items[0].kind, "MODEL");
        assert_eq!(dir.items[0].name, "stone");
        assert_eq!(dir.items[0].payload_offset, 128);
        b[64 + 19] = 255;
        assert!(Directory::read(&MemoryAssetReader(b.into())).is_err());
    }
}
