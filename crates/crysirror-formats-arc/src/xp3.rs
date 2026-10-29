// SPDX-License-Identifier: GPL-3.0-or-later
// XP3 chunk/segment layouts ported from arc_unpacker, Copyright (C) 2016 rr-.
use crate::common::*;
use anyhow::{ensure, Context, Result};
use crysirror_core::service::AssetReader;
use crysirror_resource::{
    read_all, subrange, Archive, ArchiveEntry, ArchiveFormat, DecodeContext, Probe, TransformSpec,
};
use std::{collections::BTreeMap, sync::Arc};
const MAGIC: &[u8] = b"XP3\r\n \n\x1a\x8bg\x01";
mod cxdec;
mod hxv4;
pub struct Xp3;
impl ArchiveFormat for Xp3 {
    fn id(&self) -> &'static str {
        "kirikiri/xp3"
    }
    fn probe(&self, source: &dyn AssetReader) -> Result<Probe> {
        Ok(Probe {
            confidence: if length(source)? >= MAGIC.len() as u64
                && bytes(source, 0, MAGIC.len() as u64, 11)? == MAGIC
            {
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
        let index = parse_index(source, context)?;
        if let Some(d) = index.hxv4 {
            ensure!(
                index.specials.is_empty(),
                "HXV4 archive with indirect Special descriptor"
            );
            return hxv4::open(index.archive, d, context);
        }
        if !index.specials.is_empty() || cxdec::requested(context) {
            ensure!(
                cxdec::requested(context),
                "XP3 indirect Special (filename) index needs an explicit CXDEC profile \
                 (kirikiri/xp3.cxdec_modules)"
            );
            return cxdec::open(index.archive, index.specials, context);
        }
        Ok(Box::new(index.archive))
    }
}
pub(super) struct Index {
    pub archive: ParsedArchive,
    hxv4: Option<hxv4::Descriptor>,
    specials: Vec<cxdec::Special>,
}
/// Parse the XP3 container index without applying any protection profile.
pub(super) fn parse_index(source: Arc<dyn AssetReader>, context: &DecodeContext) -> Result<Index> {
    let header = bytes(source.as_ref(), 0, 23, 23)?;
    ensure!(&header[..11] == MAGIC, "XP3 magic mismatch");
    let first = u64::from_le_bytes(header[11..19].try_into().unwrap());
    let version = if u32::from_le_bytes(header[19..23].try_into().unwrap()) == 1 {
        2
    } else {
        1
    };
    let table_offset = if version == 2 {
        let extra = bytes(source.as_ref(), first, 17, 17)?;
        u64::from_le_bytes(extra[9..17].try_into().unwrap())
    } else {
        first
    };
    let flag = bytes(source.as_ref(), table_offset, 1, 1)?[0];
    ensure!(flag <= 1, "unsupported chained/flagged XP3 index {flag:#x}");
    let sizes = bytes(
        source.as_ref(),
        table_offset + 1,
        if flag == 1 { 16 } else { 8 },
        16,
    )?;
    let packed = u64::from_le_bytes(sizes[..8].try_into().unwrap());
    let decoded = if flag == 1 {
        u64::from_le_bytes(sizes[8..16].try_into().unwrap())
    } else {
        packed
    };
    ensure!(
        decoded <= context.limits.max_index_size && packed <= context.limits.max_index_size,
        "XP3 index byte limit"
    );
    let table_reader = subrange(
        source.clone(),
        table_offset + 1 + sizes.len() as u64,
        packed,
    )?;
    let table_reader = if flag == 1 {
        context.apply(
            &TransformSpec {
                algorithm: "zlib".into(),
                key_id: None,
                parameters: serde_json::Value::Null,
                expected_size: Some(decoded),
            },
            table_reader,
            &serde_json::json!({"format":"kirikiri/xp3","scope":"index"}),
        )?
    } else {
        table_reader
    };
    let table = read_all(table_reader.as_ref(), context.limits.max_index_size)?;
    let mut c = Cursor::new(&table);
    let mut names = BTreeMap::new();
    let mut entries = Vec::new();
    let mut plans = Vec::new();
    let mut hxv4 = None;
    let mut specials = Vec::new();
    while c.left() > 0 {
        let tag = c.take(4)?;
        let size = usize::try_from(c.le64()?).context("XP3 chunk size overflow")?;
        let body = c.take(size)?;
        if tag == b"Hxv4" {
            ensure!(hxv4.is_none(), "duplicate HXV4 descriptor");
            hxv4 = Some(hxv4::descriptor(body, length(source.as_ref())?)?);
        } else if tag == b"hnfn" || tag == b"eliF" {
            let mut b = Cursor::new(body);
            let hash = b.le32()?;
            let chars = usize::from(b.le16()?);
            names.insert(hash, utf16(b.take(chars * 2)?)?);
        } else if tag == b"File" {
            limits(context, entries.len() + 1)?;
            let (entry, plan) = file_entry(
                body,
                entries.len() as u32,
                version,
                &names,
                length(source.as_ref())?,
                context,
            )?;
            entries.push(entry);
            plans.push(plan);
        } else if let Some(d) = cxdec::special_descriptor(tag, body, length(source.as_ref())?) {
            specials.push(d);
        } else {
            anyhow::bail!("unsupported XP3 index record {:?}", tag);
        }
    }
    Ok(Index {
        archive: ParsedArchive {
            format: "kirikiri/xp3",
            source,
            entries,
            plans,
        },
        hxv4,
        specials,
    })
}
fn file_entry(
    data: &[u8],
    id: u32,
    version: u32,
    names: &BTreeMap<u32, String>,
    source_size: u64,
    context: &DecodeContext,
) -> Result<(ArchiveEntry, EntryPlan)> {
    let mut c = Cursor::new(data);
    let mut info = None;
    let mut segments = None;
    let mut adler = None;
    let mut timestamp = None;
    while c.left() > 0 {
        let tag = c.take(4)?;
        let size = usize::try_from(c.le64()?).context("XP3 inner chunk size overflow")?;
        let body = c.take(size)?;
        let mut b = Cursor::new(body);
        match tag {
            b"info" => {
                ensure!(info.is_none(), "duplicate XP3 info chunk");
                let flags = b.le32()?;
                ensure!(
                    flags & 0x7fff_ffff == 0,
                    "unsupported XP3 info flags {flags:#x}"
                );
                let original = b.le64()?;
                let packed = b.le64()?;
                let chars = usize::from(b.le16()?);
                ensure!(chars <= 8192, "XP3 filename size limit");
                info = Some((flags, original, packed, utf16(b.take(chars * 2)?)?));
            }
            b"segm" => {
                ensure!(segments.is_none(), "duplicate XP3 segm chunk");
                ensure!(
                    body.len().is_multiple_of(28),
                    "invalid XP3 segment chunk size"
                );
                let count = body.len() / 28;
                limits(context, count)?;
                let mut list = Vec::with_capacity(count);
                while b.left() > 0 {
                    let flags = b.le32()?;
                    ensure!(flags & !7 == 0, "unsupported XP3 segment flags");
                    let offset = b.le64()?;
                    let size = b.le64()?;
                    let stored = b.le64()?;
                    ensure!(
                        size <= context.limits.max_entry_size,
                        "XP3 segment decoded byte limit"
                    );
                    range(source_size, offset, stored)?;
                    list.push(Segment {
                        offset,
                        stored,
                        size,
                        compressed: flags & 7 != 0,
                    });
                }
                segments = Some(list);
            }
            b"adlr" => {
                ensure!(adler.is_none(), "duplicate XP3 adlr chunk");
                adler = Some(b.le32()?);
            }
            b"time" => {
                timestamp = Some(b.le64()?);
            }
            _ => anyhow::bail!("unsupported XP3 file chunk {:?}", tag),
        }
    }
    let (flags, declared_size, declared_stored_size, name) =
        info.context("XP3 info chunk missing")?;
    let hash = adler.context("XP3 adlr chunk missing")?;
    let segments = segments.context("XP3 segm chunk missing")?;
    ensure!(!segments.is_empty(), "XP3 entry has no segments");
    let (sum_size, sum_stored) =
        segments
            .iter()
            .try_fold((0u64, 0u64), |(a, b), s| -> Result<_> {
                Ok((
                    a.checked_add(s.size).context("XP3 size sum overflow")?,
                    b.checked_add(s.stored).context("XP3 stored sum overflow")?,
                ))
            })?;
    // Upstream reads through SEGM, not the INFO size fields. Its own compressed
    // fixture stores the uncompressed size in INFO's packed-size field.
    let size = sum_size;
    let stored_size = sum_stored;
    ensure!(
        size <= context.limits.max_entry_size,
        "XP3 decoded entry byte limit"
    );
    let path = names.get(&hash).cloned().unwrap_or(name);
    let entry = ArchiveEntry {
        id,
        path,
        size,
        stored_size,
        metadata: serde_json::json!({"format":"kirikiri/xp3","version":version,"flags":flags,"adler32":hash,"timestamp":timestamp,"segment_count":segments.len(),"declared_size":declared_size,"declared_stored_size":declared_stored_size}),
    };
    Ok((
        entry,
        EntryPlan::Segments {
            segments,
            encrypted: flags & 0x8000_0000 != 0,
        },
    ))
}
