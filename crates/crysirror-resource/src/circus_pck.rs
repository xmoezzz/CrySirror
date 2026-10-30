//! Seekable Circus PCK/DAT archive index. Works independently of any VM.
// Layout reference: GARbro ArcFormats/Circus/ArcPCK.cs at
// b09ee4570ccb1daf6ac56710ee8934dc0b8baeb0, Copyright (C) 2016 morkt.
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to
// deal in the Software without restriction, including without limitation the
// rights to use, copy, modify, merge, publish, distribute, sublicense, and/or
// sell copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
use crate::{read_exact_at, subrange, Archive, ArchiveEntry, ArchiveFormat, DecodeContext, Probe};
use anyhow::{ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::AssetReader,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Count + duplicated offset/size index + 56-byte-name directory records.
pub struct CircusPck;
fn le32(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4)
            .context("Circus PCK truncated dword")?
            .try_into()?,
    ))
}
fn header(source: &dyn AssetReader) -> Result<Option<(usize, u64)>> {
    if source.len()? < 76 {
        return Ok(None);
    }
    let mut h = [0; 12];
    read_exact_at(source, 0, &mut h)?;
    let count = le32(&h, 0)? as usize;
    if count == 0 || count > 100_000 {
        return Ok(None);
    }
    let end = 4 + count as u64 * 72;
    let first = u64::from(le32(&h, 4)?);
    if first < end || first >= source.len()? {
        return Ok(None);
    }
    let mut entry = [0; 64];
    read_exact_at(source, 4 + count as u64 * 8, &mut entry)?;
    if u64::from(le32(&entry, 56)?) != first || le32(&entry, 60)? != le32(&h, 8)? || entry[0] == 0 {
        return Ok(None);
    }
    Ok(Some((count, end)))
}
impl ArchiveFormat for CircusPck {
    fn id(&self) -> &'static str {
        "circus/pck"
    }
    fn probe(&self, source: &dyn AssetReader) -> Result<Probe> {
        Ok(Probe {
            confidence: if header(source)?.is_some() { 90 } else { 0 },
        })
    }
    fn open(
        &self,
        source: Arc<dyn AssetReader>,
        context: &DecodeContext,
    ) -> Result<Box<dyn Archive>> {
        let (count, end) = header(source.as_ref())?.context("Circus PCK index layout")?;
        ensure!(
            count <= context.limits.max_entries && end <= context.limits.max_index_size,
            "Circus PCK index budget"
        );
        let mut table = vec![0; usize::try_from(end - 4)?];
        read_exact_at(source.as_ref(), 4, &mut table)?;
        let total = source.len()?;
        let mut entries = Vec::with_capacity(count);
        let mut ranges = Vec::with_capacity(count);
        let mut names = BTreeSet::new();
        let mut lookup = BTreeMap::new();
        for id in 0..count {
            let record = &table[count * 8 + id * 64..count * 8 + (id + 1) * 64];
            let nul = record[..56].iter().position(|b| *b == 0).unwrap_or(56);
            ensure!(nul > 0, "Circus PCK empty filename");
            let (decoded, errors) =
                encoding_rs::SHIFT_JIS.decode_without_bom_handling(&record[..nul]);
            ensure!(!errors, "Circus PCK filename codec");
            let path = decoded.replace('\\', "/");
            ensure!(
                VfsPath::new(Mount::Asset, &path)?.relative() == path,
                "Circus PCK noncanonical path"
            );
            ensure!(
                names.insert(path.to_ascii_lowercase()),
                "Circus PCK duplicate filename"
            );
            let offset = u64::from(le32(record, 56)?);
            let size = u64::from(le32(record, 60)?);
            ensure!(
                u64::from(le32(&table, id * 8)?) == offset
                    && u64::from(le32(&table, id * 8 + 4)?) == size,
                "Circus PCK duplicated index differs"
            );
            ensure!(
                offset >= end && offset.checked_add(size).is_some_and(|n| n <= total),
                "Circus PCK resource range"
            );
            ensure!(
                size <= context.limits.max_entry_size,
                "Circus PCK entry budget"
            );
            lookup.insert(path.to_ascii_lowercase(), id as u32);
            ranges.push((offset, size));
            entries.push(ArchiveEntry {
                id: id as u32,
                path,
                size,
                stored_size: size,
                metadata: serde_json::json!({"offset":offset,"format":self.id()}),
            });
        }
        Ok(Box::new(Pck {
            source,
            entries,
            ranges,
            lookup,
        }))
    }
}
struct Pck {
    source: Arc<dyn AssetReader>,
    entries: Vec<ArchiveEntry>,
    ranges: Vec<(u64, u64)>,
    lookup: BTreeMap<String, u32>,
}
impl Archive for Pck {
    fn entries(&self) -> &[ArchiveEntry] {
        &self.entries
    }
    fn resolve(&self, path: &str) -> Option<u32> {
        self.lookup
            .get(&path.replace('\\', "/").to_ascii_lowercase())
            .copied()
    }
    fn open(&self, id: u32, context: &DecodeContext) -> Result<Arc<dyn AssetReader>> {
        let &(offset, size) = self
            .ranges
            .get(id as usize)
            .context("Circus PCK entry id")?;
        ensure!(
            size <= context.limits.max_entry_size,
            "Circus PCK entry budget"
        );
        subrange(self.source.clone(), offset, size)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::{service::MemoryAssetReader, state::ClientId};
    fn fixture() -> Vec<u8> {
        let mut b = vec![0u8; 80];
        b[..4].copy_from_slice(&1u32.to_le_bytes());
        b[4..8].copy_from_slice(&76u32.to_le_bytes());
        b[8..12].copy_from_slice(&4u32.to_le_bytes());
        b[12..17].copy_from_slice(b"a.pcm");
        b[68..72].copy_from_slice(&76u32.to_le_bytes());
        b[72..76].copy_from_slice(&4u32.to_le_bytes());
        b[76..].copy_from_slice(b"data");
        b
    }
    #[test]
    fn bounded_case_insensitive_entry() {
        let archive = CircusPck
            .open(
                Arc::new(MemoryAssetReader(fixture().into())),
                &DecodeContext::new(ClientId(1)),
            )
            .unwrap();
        assert_eq!(archive.resolve("A.PCM"), Some(0));
        let reader = archive.open(0, &DecodeContext::new(ClientId(1))).unwrap();
        assert_eq!(crate::read_all(reader.as_ref(), 4).unwrap(), b"data");
        assert!(archive.open(1, &DecodeContext::new(ClientId(1))).is_err());
    }
    #[test]
    fn duplicated_table_and_ranges_must_agree() {
        let mut b = fixture();
        b[72..76].copy_from_slice(&99u32.to_le_bytes());
        assert!(CircusPck
            .open(
                Arc::new(MemoryAssetReader(b.into())),
                &DecodeContext::new(ClientId(1))
            )
            .is_err());
    }
    #[test]
    #[ignore = "requires owner's original locally installed Circus VOICE.DAT"]
    fn installed_voice_index_is_streamed() {
        use std::io::{Read, Seek, SeekFrom};
        struct FileReader(std::sync::Mutex<std::fs::File>, u64);
        impl AssetReader for FileReader {
            fn len(&self) -> crysirror_core::Result<u64> {
                Ok(self.1)
            }
            fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
                let mut file = self.0.lock().unwrap();
                file.seek(SeekFrom::Start(offset))
                    .map_err(|e| crysirror_core::Error::Io(e.to_string()))?;
                file.read(out)
                    .map_err(|e| crysirror_core::Error::Io(e.to_string()))
            }
        }
        let path =
            std::env::var("CRYSIRROR_CIRCUS_VOICE_DAT").expect("set CRYSIRROR_CIRCUS_VOICE_DAT");
        let file = std::fs::File::open(path).unwrap();
        let len = file.metadata().unwrap().len();
        let context = DecodeContext::new(ClientId(1));
        let archive = CircusPck
            .open(
                Arc::new(FileReader(std::sync::Mutex::new(file), len)),
                &context,
            )
            .unwrap();
        assert_eq!(archive.entries().len(), 20396);
        let id = archive.resolve("0000silent24.pcm").unwrap();
        let reader = archive.open(id, &context).unwrap();
        let mut header = [0; 4];
        read_exact_at(reader.as_ref(), 0, &mut header).unwrap();
        assert_eq!(&header, b"XPCM");
        assert!(reader.len().unwrap() < len);
    }
}
