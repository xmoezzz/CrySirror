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
mod adler_xor;
mod cxdec;
mod hash_fold_xor;
mod hxv4;
mod inline_names;
pub struct Xp3;
impl ArchiveFormat for Xp3 {
    fn id(&self) -> &'static str {
        "kirikiri/xp3"
    }
    fn probe(&self, source: &dyn AssetReader) -> Result<Probe> {
        Ok(Probe {
            confidence: if archive_base(source)?.is_some() {
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
        let archive: Box<dyn Archive> = if let Some(d) = index.hxv4 {
            ensure!(
                index.specials.is_empty(),
                "HXV4 archive with indirect Special descriptor"
            );
            hxv4::open(index.archive, d, context)?
        } else if hash_fold_xor::requested(context) {
            ensure!(
                index.specials.is_empty(),
                "hash-fold XOR cannot decode an indirect filename index"
            );
            hash_fold_xor::open(index.archive, context)?
        } else if adler_xor::requested(context) {
            ensure!(
                index.specials.is_empty(),
                "rotating Adler XOR profile cannot decode an indirect filename index"
            );
            adler_xor::open(index.archive, context)?
        } else if !index.specials.is_empty() || cxdec::requested(context) {
            ensure!(cxdec::requested(context), "XP3 indirect Special (filename) index needs an explicit CXDEC profile (kirikiri/xp3.cxdec_modules)");
            cxdec::open(index.archive, index.specials, context)?
        } else {
            Box::new(index.archive)
        };
        inline_names::publish(archive)
    }
}
// Bound-XP3 address and continuation rules follow krkrz
// fd5c4baa6a2ef5978db1bd043634351f48667daf base/XP3Archive.cpp.
// Copyright W.Dee and contributors; notice: docs/third-party/krkrz.txt.
fn archive_base(source: &dyn AssetReader) -> Result<Option<u64>> {
    let size = length(source)?;
    if size < MAGIC.len() as u64 {
        return Ok(None);
    }
    let head = bytes(source, 0, MAGIC.len() as u64, 11)?;
    if head == MAGIC {
        return Ok(Some(0));
    }
    if !head.starts_with(b"MZ") {
        return Ok(None);
    }
    // Bounded individual reads; aligned candidates include an overlap at the
    // end of each chunk. No PE instructions or executable resources are run.
    const CHUNK: u64 = 256 * 1024;
    let mut at = 16;
    while at < size {
        let count = (size - at).min(CHUNK + 10);
        let data = bytes(source, at, count, CHUNK + 10)?;
        for p in (0..data.len().saturating_sub(10)).step_by(16) {
            if data[p..].starts_with(MAGIC) {
                return Ok(Some(at + p as u64));
            }
        }
        at = at.checked_add(CHUNK).context("XP3 scan address overflow")?;
    }
    Ok(None)
}
fn read_index_chain(
    source: Arc<dyn AssetReader>,
    mut at: u64,
    base: u64,
    context: &DecodeContext,
) -> Result<Vec<u8>> {
    let mut table = Vec::new();
    let mut visited = std::collections::BTreeSet::new();
    loop {
        ensure!(
            visited.len() < 1024 && visited.insert(at),
            "XP3 index continuation cycle/block limit"
        );
        let flag = bytes(source.as_ref(), at, 1, 1)?[0];
        let method = flag & 0x7f;
        ensure!(method <= 1, "unsupported XP3 index method {flag:#x}");
        let sizes = bytes(
            source.as_ref(),
            at + 1,
            if method == 1 { 16 } else { 8 },
            16,
        )?;
        let packed = u64::from_le_bytes(sizes[..8].try_into().unwrap());
        let decoded = if method == 1 {
            u64::from_le_bytes(sizes[8..16].try_into().unwrap())
        } else {
            packed
        };
        ensure!(
            decoded
                <= context
                    .limits
                    .max_index_size
                    .saturating_sub(table.len() as u64)
                && packed <= context.limits.max_index_size,
            "XP3 index byte limit"
        );
        let body_at = at
            .checked_add(1 + sizes.len() as u64)
            .context("XP3 index address overflow")?;
        let reader = subrange(source.clone(), body_at, packed)?;
        let reader = if method == 1 {
            context.apply(
                &TransformSpec {
                    algorithm: "zlib".into(),
                    key_id: None,
                    parameters: serde_json::Value::Null,
                    expected_size: Some(decoded),
                },
                reader,
                &serde_json::json!({"format":"kirikiri/xp3","scope":"index"}),
            )?
        } else {
            reader
        };
        table.extend(read_all(reader.as_ref(), context.limits.max_index_size)?);
        if flag & 0x80 == 0 {
            break;
        }
        let next_at = body_at
            .checked_add(packed)
            .context("XP3 index address overflow")?;
        let next = bytes(source.as_ref(), next_at, 8, 8)?;
        at = base.wrapping_add(u64::from_le_bytes(next.try_into().unwrap()));
    }
    Ok(table)
}
pub(super) struct Index {
    pub archive: ParsedArchive,
    hxv4: Option<hxv4::Descriptor>,
    specials: Vec<cxdec::Special>,
}
/// Parse the XP3 container index without applying any protection profile.
pub(super) fn parse_index(source: Arc<dyn AssetReader>, context: &DecodeContext) -> Result<Index> {
    let base = archive_base(source.as_ref())?.context("XP3 magic mismatch")?;
    let header = bytes(source.as_ref(), base, 23, 23)?;
    // Native unsigned address addition intentionally accepts a negative relative
    // offset: bound archives can place a continuation index before their mark.
    let first = base.wrapping_add(u64::from_le_bytes(header[11..19].try_into().unwrap()));
    let version = if u32::from_le_bytes(header[19..23].try_into().unwrap()) == 1 {
        2
    } else {
        1
    };
    let table_offset = if version == 2 {
        let extra = bytes(source.as_ref(), first, 17, 17)?;
        base.wrapping_add(u64::from_le_bytes(extra[9..17].try_into().unwrap()))
    } else {
        first
    };
    let table = read_index_chain(source.clone(), table_offset, base, context)?;
    let mut c = Cursor::new(&table);
    let mut names = BTreeMap::new();
    let mut entries = Vec::new();
    let mut plans = Vec::new();
    let mut hxv4 = None;
    let mut specials = Vec::new();
    let mut inline_name = None;
    while c.left() > 0 {
        let tag = c.take(4)?;
        let size = usize::try_from(c.le64()?).context("XP3 chunk size overflow")?;
        let body = c.take(size)?;
        ensure!(
            inline_name.is_none() || tag == b"File",
            "XP3 inline filename must immediately precede File"
        );
        if tag == b"Hxv4" {
            ensure!(
                base == 0,
                "bound XP3 HXV4 descriptor addressing is unsupported"
            );
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
                inline_name.take(),
                length(source.as_ref())?,
                base,
                context,
            )?;
            entries.push(entry);
            plans.push(plan);
        } else if let Some(d) = cxdec::special_descriptor(tag, body, length(source.as_ref())?) {
            ensure!(
                base == 0,
                "bound XP3 indirect descriptor addressing is unsupported"
            );
            specials.push(d);
        } else if let Some(name) = parse_inline_name(tag, body)? {
            inline_name = Some(name);
        } else {
            anyhow::bail!("unsupported XP3 index record {:?}", tag);
        }
    }
    ensure!(inline_name.is_none(), "XP3 dangling inline filename");
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
/// M2/KrkrZ inline name facts from xp3-brute c82cb14302138125640478556723705aa720007b,
/// xp3.rs classify_root/parse_root_chunks. The FourCC is not a game selector.
struct InlineName {
    hash: u32,
    name: String,
}
fn parse_inline_name(_tag: &[u8], body: &[u8]) -> Result<Option<InlineName>> {
    if body.len() < 6 {
        return Ok(None);
    }
    let chars = usize::from(u16::from_le_bytes(body[4..6].try_into().unwrap()));
    if chars == 0 || chars > 4096 {
        return Ok(None);
    }
    let end = 6 + chars * 2;
    if end != body.len() && !(end + 2 == body.len() && body[end..] == [0, 0]) {
        return Ok(None);
    }
    let name = utf16(&body[6..end])?;
    ensure!(
        !name.chars().any(|c| c.is_control()),
        "XP3 inline filename control character"
    );
    let canonical = name.replace('\\', "/");
    crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, &canonical)
        .map_err(|e| anyhow::anyhow!("XP3 inline filename: {e}"))?;
    Ok(Some(InlineName {
        hash: u32::from_le_bytes(body[..4].try_into().unwrap()),
        name,
    }))
}

fn file_entry(
    data: &[u8],
    id: u32,
    version: u32,
    names: &BTreeMap<u32, String>,
    inline_name: Option<InlineName>,
    source_size: u64,
    base: u64,
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
                    let offset = base.wrapping_add(b.le64()?);
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
    let native_name = name.clone();
    let has_inline = inline_name.is_some();
    let path = if let Some(inline) = inline_name {
        ensure!(
            inline.hash == hash,
            "XP3 inline filename hash does not match File adlr"
        );
        let direct = name == inline.name;
        let token = name.len() == 32
            && name.bytes().all(|b| b.is_ascii_hexdigit())
            && name.eq_ignore_ascii_case(&cxdec::filename_token(&inline.name));
        ensure!(
            direct || token,
            "XP3 inline filename does not match original info name/token"
        );
        inline.name.replace('\\', "/")
    } else {
        names.get(&hash).cloned().unwrap_or(name)
    };
    let mut entry = ArchiveEntry {
        id,
        path,
        size,
        stored_size,
        metadata: serde_json::json!({"format":"kirikiri/xp3","version":version,"flags":flags,"adler32":hash,"timestamp":timestamp,"segment_count":segments.len(),"declared_size":declared_size,"declared_stored_size":declared_stored_size}),
    };
    if has_inline {
        entry.metadata["xp3_inline_name"] = serde_json::json!(entry.path);
        entry.metadata["xp3_info_name"] = serde_json::json!(native_name);
    }
    Ok((
        entry,
        EntryPlan::Segments {
            segments,
            encrypted: flags & 0x8000_0000 != 0,
        },
    ))
}
