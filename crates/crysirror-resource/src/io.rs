// SPDX-License-Identifier: MPL-2.0
use anyhow::{Context, Result, ensure};
use crysirror_core::service::{AssetReader, MemoryAssetReader};
use std::sync::Arc;

pub fn memory(bytes: Vec<u8>) -> Arc<dyn AssetReader> {
    Arc::new(MemoryAssetReader(Arc::from(bytes)))
}
pub fn read_exact_at(source: &dyn AssetReader, offset: u64, out: &mut [u8]) -> Result<()> {
    let mut done = 0;
    while done < out.len() {
        let at = offset
            .checked_add(done as u64)
            .context("resource offset overflow")?;
        let n = source
            .read_at(at, &mut out[done..])
            .map_err(|e| anyhow::anyhow!("resource read: {e}"))?;
        ensure!(
            n > 0 && n <= out.len() - done,
            "short or invalid resource read"
        );
        done += n;
    }
    Ok(())
}
pub fn read_all(source: &dyn AssetReader, limit: u64) -> Result<Vec<u8>> {
    let len = source
        .len()
        .map_err(|e| anyhow::anyhow!("resource size: {e}"))?;
    ensure!(len <= limit, "resource exceeds byte limit");
    let len = usize::try_from(len).context("resource exceeds address space")?;
    let mut out = Vec::new();
    out.try_reserve_exact(len)?;
    out.resize(len, 0);
    read_exact_at(source, 0, &mut out)?;
    Ok(out)
}
struct SubReader {
    source: Arc<dyn AssetReader>,
    offset: u64,
    len: u64,
}
impl AssetReader for SubReader {
    fn len(&self) -> crysirror_core::Result<u64> {
        Ok(self.len)
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        if offset >= self.len {
            return Ok(0);
        }
        let count = out
            .len()
            .min((self.len - offset).min(usize::MAX as u64) as usize);
        self.source.read_at(self.offset + offset, &mut out[..count])
    }
}
pub fn subrange(
    source: Arc<dyn AssetReader>,
    offset: u64,
    len: u64,
) -> Result<Arc<dyn AssetReader>> {
    ensure!(
        offset
            .checked_add(len)
            .is_some_and(|end| source.len().is_ok_and(|n| end <= n)),
        "resource range outside source"
    );
    Ok(Arc::new(SubReader {
        source,
        offset,
        len,
    }))
}
struct ConcatReader {
    parts: Vec<(u64, u64, Arc<dyn AssetReader>)>,
    len: u64,
}
impl AssetReader for ConcatReader {
    fn len(&self) -> crysirror_core::Result<u64> {
        Ok(self.len)
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        if offset >= self.len || out.is_empty() {
            return Ok(0);
        }
        let mut done = 0;
        for (start, len, source) in &self.parts {
            let at = offset + done as u64;
            if at < *start || at >= start + len {
                continue;
            }
            let local = at - start;
            let n = out.len() - done;
            let count = n.min((len - local).min(usize::MAX as u64) as usize);
            let mut read = 0;
            while read < count {
                let n = source.read_at(local + read as u64, &mut out[done + read..done + count])?;
                if n == 0 || n > count - read {
                    return Err(crysirror_core::Error::Io("short archive segment".into()));
                }
                read += n;
            }
            done += count;
            if done == out.len() {
                break;
            }
        }
        Ok(done)
    }
}
pub fn concat(sources: Vec<Arc<dyn AssetReader>>) -> Result<Arc<dyn AssetReader>> {
    let mut len = 0u64;
    let mut parts = Vec::new();
    for source in sources {
        let size = source.len().map_err(|e| anyhow::anyhow!("{e}"))?;
        parts.push((len, size, source));
        len = len
            .checked_add(size)
            .context("archive segment sizes overflow")?;
    }
    Ok(Arc::new(ConcatReader { parts, len }))
}
pub use crysirror_io::AssetIo;
