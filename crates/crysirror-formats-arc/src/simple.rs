// SPDX-License-Identifier: GPL-3.0-or-later
// Layouts ported from arc_unpacker, Copyright (C) 2016 rr-.
use crate::common::*;
use anyhow::{ensure, Result};
use crysirror_core::service::AssetReader;
use crysirror_resource::{Archive, ArchiveEntry, ArchiveFormat, DecodeContext, Probe};
use std::sync::Arc;

pub struct BgiArc;
impl ArchiveFormat for BgiArc {
    fn id(&self) -> &'static str {
        "bgi/arc"
    }
    fn probe(&self, source: &dyn AssetReader) -> Result<Probe> {
        if length(source)? < 12 {
            return Ok(Probe { confidence: 0 });
        }
        let m = bytes(source, 0, 12, 12)?;
        Ok(Probe {
            confidence: if m == b"PackFile    " || m == b"BURIKO ARC20" {
                100
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
        let header = bytes(source.as_ref(), 0, 16, 16)?;
        let mut c = Cursor::new(&header);
        let magic = c.take(12)?;
        let (name_len, skip) = if magic == b"PackFile    " {
            (16usize, 8usize)
        } else {
            ensure!(magic == b"BURIKO ARC20", "unsupported BGI archive header");
            (96, 24)
        };
        let count = c.le32()? as usize;
        limits(context, count)?;
        let table_size = (count as u64)
            .checked_mul((name_len + 8 + skip) as u64)
            .ok_or_else(|| anyhow::anyhow!("BGI table size overflow"))?;
        let table = bytes(
            source.as_ref(),
            16,
            table_size,
            context.limits.max_index_size,
        )?;
        let data_start = 16 + table_size;
        let source_size = length(source.as_ref())?;
        let mut c = Cursor::new(&table);
        let mut entries = Vec::with_capacity(count);
        let mut plans = Vec::with_capacity(count);
        for id in 0..count {
            let raw = c.take(name_len)?;
            let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
            let path = sjis(&raw[..end])?;
            let offset = data_start + u64::from(c.le32()?);
            let size = u64::from(c.le32()?);
            c.take(skip)?;
            range(source_size, offset, size)?;
            ensure!(
                size <= context.limits.max_entry_size,
                "BGI entry byte limit"
            );
            entries.push(ArchiveEntry{id:id as u32,path,size,stored_size:size,metadata:serde_json::json!({"format":self.id(),"variant":if name_len==16{"packfile"}else{"arc20"},"offset":offset})});
            plans.push(EntryPlan::Raw { offset });
        }
        Ok(Box::new(ParsedArchive {
            format: self.id(),
            source,
            entries,
            plans,
        }))
    }
}
pub struct MajiroArc;
impl ArchiveFormat for MajiroArc {
    fn id(&self) -> &'static str {
        "majiro/arc"
    }
    fn probe(&self, source: &dyn AssetReader) -> Result<Probe> {
        Ok(Probe {
            confidence: if length(source)? >= 10 && bytes(source, 0, 10, 10)? == b"MajiroArcV" {
                100
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
        let header = bytes(source.as_ref(), 0, 28, 28)?;
        let mut c = Cursor::new(&header);
        let magic = c.take(16)?;
        let version = if magic == b"MajiroArcV2.000\0" {
            2
        } else if magic == b"MajiroArcV3.000\0" {
            3
        } else {
            anyhow::bail!("unsupported Majiro archive version")
        };
        let count = c.le32()? as usize;
        limits(context, count)?;
        let names = u64::from(c.le32()?);
        let data_offset = u64::from(c.le32()?);
        let width = if version == 3 { 16u64 } else { 12 };
        let table_size = count as u64 * width;
        let source_size = length(source.as_ref())?;
        ensure!(
            names >= 28 + table_size && names <= source_size,
            "invalid Majiro names offset"
        );
        ensure!(
            data_offset >= names && data_offset <= source_size,
            "invalid Majiro data offset"
        );
        let table = bytes(
            source.as_ref(),
            28,
            table_size,
            context.limits.max_index_size,
        )?;
        let name_data = bytes(
            source.as_ref(),
            names,
            data_offset - names,
            context.limits.max_index_size.saturating_sub(table_size),
        )?;
        let mut c = Cursor::new(&table);
        let mut names = Cursor::new(&name_data);
        let mut entries = Vec::with_capacity(count);
        let mut plans = Vec::with_capacity(count);
        for id in 0..count {
            let hash = if version == 3 {
                c.le64()?
            } else {
                u64::from(c.le32()?)
            };
            let offset = u64::from(c.le32()?);
            let size = u64::from(c.le32()?);
            range(source_size, offset, size)?;
            ensure!(offset >= data_offset, "Majiro entry overlaps index");
            ensure!(
                size <= context.limits.max_entry_size,
                "Majiro entry byte limit"
            );
            entries.push(ArchiveEntry{id:id as u32,path:sjis(names.zero()?)?,size,stored_size:size,metadata:serde_json::json!({"format":self.id(),"version":version,"hash":hash,"offset":offset})});
            plans.push(EntryPlan::Raw { offset });
        }
        Ok(Box::new(ParsedArchive {
            format: self.id(),
            source,
            entries,
            plans,
        }))
    }
}
