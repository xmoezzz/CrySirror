// SPDX-License-Identifier: GPL-3.0-or-later
// Archive layouts adapted from arc_unpacker, Copyright (C) 2016 rr-.
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::service::AssetReader;
use crysirror_resource::{
    concat, read_all, read_exact_at, subrange, Archive, ArchiveEntry, DecodeContext, TransformSpec,
};
use std::sync::Arc;

pub(crate) fn length(source: &dyn AssetReader) -> Result<u64> {
    source.len().map_err(|e| anyhow::anyhow!("{e}"))
}
pub(crate) fn bytes(
    source: &dyn AssetReader,
    offset: u64,
    size: u64,
    limit: u64,
) -> Result<Vec<u8>> {
    ensure!(size <= limit, "archive index/read byte limit");
    let source_size = length(source)?;
    ensure!(
        offset
            .checked_add(size)
            .is_some_and(|end| end <= source_size),
        "archive data range outside source"
    );
    let mut out = vec![0; usize::try_from(size).context("archive range exceeds address space")?];
    read_exact_at(source, offset, &mut out)?;
    Ok(out)
}
pub(crate) fn range(source_size: u64, offset: u64, size: u64) -> Result<()> {
    ensure!(
        offset
            .checked_add(size)
            .is_some_and(|end| end <= source_size),
        "archive entry range outside source"
    );
    Ok(())
}
pub(crate) struct Cursor<'a> {
    pub data: &'a [u8],
    pub at: usize,
}
impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }
    pub fn left(&self) -> usize {
        self.data.len() - self.at
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).context("archive cursor overflow")?;
        ensure!(end <= self.data.len(), "truncated archive table");
        let out = &self.data[self.at..end];
        self.at = end;
        Ok(out)
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn le16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn le32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn be32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn le64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn zero(&mut self) -> Result<&'a [u8]> {
        let size = self.data[self.at..]
            .iter()
            .position(|b| *b == 0)
            .context("unterminated archive filename")?;
        ensure!(size <= 16 * 1024, "archive filename length limit");
        let out = self.take(size)?;
        self.take(1)?;
        Ok(out)
    }
}
pub(crate) fn sjis(bytes: &[u8]) -> Result<String> {
    let (text, errors) = encoding_rs::SHIFT_JIS.decode_without_bom_handling(bytes);
    ensure!(!errors, "invalid CP932 archive filename");
    Ok(text.into_owned())
}
pub(crate) fn utf16(bytes: &[u8]) -> Result<String> {
    ensure!(bytes.len().is_multiple_of(2), "odd UTF16 filename size");
    let units: Vec<_> = bytes
        .chunks_exact(2)
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .collect();
    String::from_utf16(&units).context("invalid UTF16 archive filename")
}
pub(crate) fn limits(context: &DecodeContext, count: usize) -> Result<()> {
    ensure!(
        count <= context.limits.max_entries && count <= u32::MAX as usize,
        "archive entry count limit"
    );
    Ok(())
}
#[derive(Clone)]
pub(crate) struct Segment {
    pub offset: u64,
    pub stored: u64,
    pub size: u64,
    pub compressed: bool,
}
pub(crate) enum EntryPlan {
    Raw {
        offset: u64,
    },
    Segments {
        segments: Vec<Segment>,
        encrypted: bool,
    },
    NsaLzss {
        offset: u64,
    },
    Unsupported(&'static str),
}
pub(crate) struct ParsedArchive {
    pub format: &'static str,
    pub source: Arc<dyn AssetReader>,
    pub entries: Vec<ArchiveEntry>,
    pub plans: Vec<EntryPlan>,
}
impl Archive for ParsedArchive {
    fn entries(&self) -> &[ArchiveEntry] {
        &self.entries
    }
    fn open(&self, id: u32, context: &DecodeContext) -> Result<Arc<dyn AssetReader>> {
        let entry = self
            .entries
            .get(id as usize)
            .filter(|e| e.id == id)
            .context("archive entry id does not exist")?;
        ensure!(
            entry.size <= context.limits.max_entry_size,
            "archive entry decoded byte limit"
        );
        let (data, required) = match &self.plans[id as usize] {
            EntryPlan::Raw { offset } => (
                subrange(self.source.clone(), *offset, entry.stored_size)?,
                false,
            ),
            EntryPlan::Segments {
                segments,
                encrypted,
            } => {
                let mut readers = Vec::with_capacity(segments.len());
                for segment in segments {
                    let source = subrange(self.source.clone(), segment.offset, segment.stored)?;
                    let reader = if segment.compressed {
                        context.apply(
                            &TransformSpec {
                                algorithm: "zlib".into(),
                                key_id: None,
                                parameters: serde_json::Value::Null,
                                expected_size: Some(segment.size),
                            },
                            source,
                            &entry.metadata,
                        )?
                    } else {
                        ensure!(
                            segment.stored == segment.size,
                            "plain segment size mismatch"
                        );
                        source
                    };
                    readers.push(reader);
                }
                (concat(readers)?, *encrypted)
            }
            EntryPlan::NsaLzss { offset } => {
                let input = read_all(
                    subrange(self.source.clone(), *offset, entry.stored_size)?.as_ref(),
                    context.limits.max_resident_bytes,
                )?;
                (
                    context.decode_with(entry.size, || nsa_lzss(&input, entry.size))?,
                    false,
                )
            }
            EntryPlan::Unsupported(reason) => bail!("{reason}"),
        };
        context.apply_pipeline(self.format, entry, data, required)
    }
}
/// NScripter's MSB-first bitwise LZSS: 8-bit dictionary address, 4-bit length+2,
/// zero-filled 256-byte ring, initial write position 239. Overlap is intentional.
fn nsa_lzss(input: &[u8], size: u64) -> Result<Vec<u8>> {
    let n = usize::try_from(size).context("LZSS output size overflow")?;
    let mut out = Vec::with_capacity(n);
    let mut ring = [0u8; 256];
    let mut write = 239usize;
    let mut bit = 0usize;
    let mut get = |count: usize| -> Result<u32> {
        ensure!(
            bit.checked_add(count)
                .is_some_and(|end| end <= input.len().saturating_mul(8)),
            "truncated NScripter LZSS bitstream"
        );
        let mut value = 0;
        for _ in 0..count {
            value = (value << 1) | u32::from((input[bit / 8] >> (7 - bit % 8)) & 1);
            bit += 1;
        }
        Ok(value)
    };
    while out.len() < n {
        if get(1)? != 0 {
            let value = get(8)? as u8;
            out.push(value);
            ring[write] = value;
            write = (write + 1) & 255;
        } else {
            let mut read = get(8)? as usize;
            let count = get(4)? as usize + 2;
            for _ in 0..count.min(n - out.len()) {
                let value = ring[read];
                read = (read + 1) & 255;
                out.push(value);
                ring[write] = value;
                write = (write + 1) & 255;
            }
        }
    }
    Ok(out)
}
