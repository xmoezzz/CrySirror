// SPDX-License-Identifier: GPL-3.0-or-later
// NScripter archive layouts and LZSS parameters: arc_unpacker, Copyright (C) 2016 rr-.
use crate::common::*;
use anyhow::{ensure, Result};
use crysirror_core::{service::AssetReader, state::ClientId};
use crysirror_resource::{Archive, ArchiveEntry, ArchiveFormat, DecodeContext, Probe};
use std::sync::Arc;

/// These headers do not declare a filename codec. Selection is explicit;
/// Japanese CP932 is the plugin default, UTF-8 can be selected by its caller.
#[derive(Clone, Copy, Debug)]
pub enum FilenameEncoding {
    ShiftJis,
    Utf8,
}
pub struct Nscripter {
    pub compressed: bool,
    pub filename_encoding: FilenameEncoding,
}
impl Nscripter {
    pub fn sar() -> Self {
        Self {
            compressed: false,
            filename_encoding: FilenameEncoding::ShiftJis,
        }
    }
    pub fn nsa() -> Self {
        Self {
            compressed: true,
            filename_encoding: FilenameEncoding::ShiftJis,
        }
    }
}
impl ArchiveFormat for Nscripter {
    fn id(&self) -> &'static str {
        if self.compressed {
            "nscripter/nsa"
        } else {
            "nscripter/sar"
        }
    }
    fn probe(&self, source: &dyn AssetReader) -> Result<Probe> {
        let size = length(source)?;
        if size < 6 {
            return Ok(Probe { confidence: 0 });
        }
        let header = bytes(source, 0, 6, 6)?;
        let count = u16::from_be_bytes(header[..2].try_into().unwrap()) as usize;
        let offset = u64::from(u32::from_be_bytes(header[2..6].try_into().unwrap()));
        if count == 0
            || offset > size
            || offset < 6 + count as u64 * if self.compressed { 14 } else { 9 }
        {
            return Ok(Probe { confidence: 0 });
        }
        if offset > 256 * 1024 {
            return Ok(Probe { confidence: 0 });
        }
        let mut context = DecodeContext::new(ClientId(0));
        context.limits.max_index_size = 256 * 1024;
        let table = bytes(source, 6, offset - 6, context.limits.max_index_size)?;
        let ok = self
            .parse_count(&table, offset, size, &context, count)
            .is_ok();
        Ok(Probe {
            confidence: if ok {
                if self.compressed {
                    38
                } else {
                    35
                }
            } else {
                0
            },
        })
    }
    fn open(
        &self,
        source: Arc<dyn AssetReader>,
        context: &DecodeContext,
    ) -> Result<Box<dyn Archive>> {
        let header = bytes(source.as_ref(), 0, 6, 6)?;
        let count = u16::from_be_bytes(header[..2].try_into().unwrap()) as usize;
        limits(context, count)?;
        let offset = u64::from(u32::from_be_bytes(header[2..6].try_into().unwrap()));
        ensure!(offset >= 6, "invalid NScripter data offset");
        let table = bytes(
            source.as_ref(),
            6,
            offset - 6,
            context.limits.max_index_size,
        )?;
        let (entries, plans) =
            self.parse_count(&table, offset, length(source.as_ref())?, context, count)?;
        Ok(Box::new(ParsedArchive {
            format: self.id(),
            source,
            entries,
            plans,
        }))
    }
}
impl Nscripter {
    fn parse_count(
        &self,
        table: &[u8],
        offset: u64,
        size: u64,
        context: &DecodeContext,
        count: usize,
    ) -> Result<(Vec<ArchiveEntry>, Vec<EntryPlan>)> {
        let mut c = Cursor::new(table);
        let mut entries = Vec::with_capacity(count);
        let mut plans = Vec::with_capacity(count);
        for id in 0..count {
            let raw = c.zero()?;
            ensure!(!raw.is_empty(), "empty NScripter filename");
            let path = match self.filename_encoding {
                FilenameEncoding::ShiftJis => sjis(raw)?,
                FilenameEncoding::Utf8 => std::str::from_utf8(raw)?.to_owned(),
            };
            let kind = if self.compressed { c.u8()? } else { 0 };
            ensure!(kind <= 2, "unsupported NSA compression type {kind}");
            let relative = u64::from(c.be32()?);
            let stored = u64::from(c.be32()?);
            let decoded = if self.compressed {
                u64::from(c.be32()?)
            } else {
                stored
            };
            let entry_offset = offset
                .checked_add(relative)
                .ok_or_else(|| anyhow::anyhow!("NScripter offset overflow"))?;
            range(size, entry_offset, stored)?;
            ensure!(
                decoded <= context.limits.max_entry_size,
                "NScripter decoded entry limit"
            );
            let plan=match kind{0=>{ensure!(stored==decoded,"uncompressed NSA size mismatch");EntryPlan::Raw{offset:entry_offset}},2=>EntryPlan::NsaLzss{offset:entry_offset},_=>EntryPlan::Unsupported("NSA SPB image conversion requires an explicit image codec; archive extraction does not silently return compressed bytes")};
            entries.push(ArchiveEntry{id:id as u32,path,size:decoded,stored_size:stored,metadata:serde_json::json!({"format":self.id(),"offset":entry_offset,"compression":kind,"filename_encoding":match self.filename_encoding{FilenameEncoding::ShiftJis=>"shift_jis",FilenameEncoding::Utf8=>"utf8"}})});
            plans.push(plan);
        }
        ensure!(
            c.data[c.at..].iter().all(|b| *b == 0),
            "unexpected bytes after NScripter table"
        );
        Ok((entries, plans))
    }
}
