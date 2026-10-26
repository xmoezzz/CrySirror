// SPDX-License-Identifier: GPL-3.0-or-later
// Archive layouts ported from arc_unpacker, Copyright (C) 2016 rr-.
//! Table-driven legacy archive plugins. Each format only parses its native
//! index; storage, ranges, budgets and decompression are shared engine paths.
use crate::common::*;
use anyhow::{ensure, Context, Result};
use crysirror_core::service::AssetReader;
use crysirror_resource::{
    Archive, ArchiveEntry, ArchiveFormat, DecodeContext, Probe, TransformSpec,
};
use std::{io::Read, sync::Arc};

mod a;
mod b;
mod c;
mod d;
mod e;
mod f;
mod g;
mod h;
mod i;

/// How a parsed entry's stored bytes become its logical bytes.
pub(crate) enum Plan {
    Raw,
    Zlib,
    Lzss(serde_json::Value),
    Decode(DecodeFn),
    /// Caller-selected decrypt/decompress chain produces the logical bytes.
    Profile,
    Unsupported(&'static str),
}
pub(crate) struct Row {
    pub path: String,
    pub offset: u64,
    pub stored: u64,
    pub size: u64,
    pub plan: Plan,
    pub meta: serde_json::Value,
}
impl Row {
    pub fn raw(path: String, offset: u64, size: u64) -> Self {
        Self {
            path,
            offset,
            stored: size,
            size,
            plan: Plan::Raw,
            meta: serde_json::Value::Null,
        }
    }
    pub fn packed(path: String, offset: u64, stored: u64, size: u64, plan: Plan) -> Self {
        Self {
            path,
            offset,
            stored,
            size,
            plan,
            meta: serde_json::Value::Null,
        }
    }
    pub fn meta(mut self, meta: serde_json::Value) -> Self {
        self.meta = meta;
        self
    }
}
type OpenFn = fn(&dyn AssetReader, &DecodeContext) -> Result<Vec<Row>>;
type ProbeFn = fn(&dyn AssetReader) -> Result<bool>;
pub(crate) struct Legacy {
    pub id: &'static str,
    /// `None`: the native format has no signature (the upstream tool keyed on
    /// filename extension); callers must select the format explicitly.
    pub probe: Option<ProbeFn>,
    pub open: OpenFn,
}
impl ArchiveFormat for Legacy {
    fn id(&self) -> &'static str {
        self.id
    }
    fn probe(&self, source: &dyn AssetReader) -> Result<Probe> {
        let Some(probe) = self.probe else {
            return Ok(Probe::NO);
        };
        // Structural probes read native tables; malformed data is a non-match.
        Ok(if probe(source).unwrap_or(false) {
            Probe::YES
        } else {
            Probe::NO
        })
    }
    fn open(
        &self,
        source: Arc<dyn AssetReader>,
        context: &DecodeContext,
    ) -> Result<Box<dyn Archive>> {
        let rows = (self.open)(source.as_ref(), context)?;
        finish(self.id, source, context, rows)
    }
}
fn finish(
    format: &'static str,
    source: Arc<dyn AssetReader>,
    context: &DecodeContext,
    rows: Vec<Row>,
) -> Result<Box<dyn Archive>> {
    limits(context, rows.len())?;
    let source_size = length(source.as_ref())?;
    let mut entries = Vec::with_capacity(rows.len());
    let mut plans = Vec::with_capacity(rows.len());
    for (id, row) in rows.into_iter().enumerate() {
        range(source_size, row.offset, row.stored)?;
        ensure!(
            row.size <= context.limits.max_entry_size,
            "{format} entry byte limit"
        );
        let mut metadata = serde_json::json!({"format": format, "offset": row.offset});
        if let serde_json::Value::Object(extra) = row.meta {
            metadata.as_object_mut().unwrap().extend(extra);
        }
        let spec = |algorithm: &str, parameters| TransformSpec {
            algorithm: algorithm.into(),
            key_id: None,
            parameters,
            expected_size: Some(row.size),
        };
        plans.push(match row.plan {
            Plan::Raw => {
                ensure!(row.stored == row.size, "{format} stored size mismatch");
                EntryPlan::Raw { offset: row.offset }
            }
            Plan::Zlib => EntryPlan::Transform {
                offset: row.offset,
                specs: vec![spec("zlib", serde_json::Value::Null)],
            },
            Plan::Lzss(parameters) => EntryPlan::Transform {
                offset: row.offset,
                specs: vec![spec("lzss", parameters)],
            },
            Plan::Decode(decode) => EntryPlan::Decode {
                offset: row.offset,
                decode,
            },
            Plan::Profile => EntryPlan::Profile { offset: row.offset },
            Plan::Unsupported(reason) => EntryPlan::Unsupported(reason),
        });
        entries.push(ArchiveEntry {
            id: id as u32,
            path: row.path,
            size: row.size,
            stored_size: row.stored,
            metadata,
        });
    }
    Ok(Box::new(ParsedArchive {
        format,
        source,
        entries,
        plans,
    }))
}
pub(crate) fn formats() -> impl Iterator<Item = &'static Legacy> {
    a::FORMATS
        .iter()
        .chain(b::FORMATS)
        .chain(c::FORMATS)
        .chain(d::FORMATS)
        .chain(e::FORMATS)
        .chain(f::FORMATS)
        .chain(g::FORMATS)
        .chain(h::FORMATS)
        .chain(i::FORMATS)
}
pub(crate) fn magic(source: &dyn AssetReader, at: u64, expected: &[u8]) -> Result<bool> {
    let end = at + expected.len() as u64;
    Ok(length(source)? >= end && bytes(source, at, expected.len() as u64, 64)? == expected)
}
/// Index bytes from `offset` to the end of the source, capped by the index
/// budget, for formats whose table length is only known by parsing it.
pub(crate) fn window(
    source: &dyn AssetReader,
    context: &DecodeContext,
    offset: u64,
) -> Result<Vec<u8>> {
    let size = length(source)?;
    ensure!(offset <= size, "archive table offset outside source");
    bytes(
        source,
        offset,
        (size - offset).min(context.limits.max_index_size),
        context.limits.max_index_size,
    )
}
/// Shared bytewise LZSS for compressed native tables.
pub(crate) fn lzss_index(
    data: &[u8],
    size: u64,
    context: &DecodeContext,
    invert_length: bool,
) -> Result<Vec<u8>> {
    ensure!(
        size <= context.limits.max_index_size,
        "archive index byte limit"
    );
    crysirror_resource::lzss_decompress(
        data,
        size as usize,
        &crysirror_resource::LzssParams {
            invert_length,
            ..Default::default()
        },
    )
}
/// Upstream names unnamed entries `unk_<n>.dat`, zero-padded to the entry
/// count's digit width (`unk.dat` for a single entry).
pub(crate) fn name_unnamed(rows: &mut [Row]) {
    let width = if rows.len() > 1 {
        rows.len().ilog10() as usize + 1
    } else {
        0
    };
    let single = rows.len() == 1;
    for (n, row) in rows.iter_mut().filter(|r| r.path.is_empty()).enumerate() {
        row.path = if single {
            "unk.dat".into()
        } else {
            format!("unk_{n:0width$}.dat")
        };
    }
}
pub(crate) fn index(
    source: &dyn AssetReader,
    context: &DecodeContext,
    offset: u64,
    size: u64,
) -> Result<Vec<u8>> {
    bytes(source, offset, size, context.limits.max_index_size)
}
pub(crate) fn inflate_index(data: &[u8], limit: u64) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .take(limit + 1)
        .read_to_end(&mut out)
        .context("zlib archive index")?;
    ensure!(out.len() as u64 <= limit, "archive index byte limit");
    Ok(out)
}
/// Fixed-size field truncated at its first NUL (upstream `read_to_zero(n)`).
pub(crate) fn zstr(raw: &[u8]) -> &[u8] {
    &raw[..raw.iter().position(|b| *b == 0).unwrap_or(raw.len())]
}
pub(crate) fn ascii(raw: &[u8]) -> Result<String> {
    ensure!(raw.is_ascii(), "non-ASCII archive filename");
    Ok(String::from_utf8(raw.to_vec())?)
}
pub(crate) fn has_ext(path: &str, ext: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, e)| e.eq_ignore_ascii_case(ext))
}
pub(crate) fn decode(
    f: impl Fn(&[u8], &ArchiveEntry) -> Result<Vec<u8>> + Send + Sync + 'static,
) -> Plan {
    Plan::Decode(Arc::new(f))
}
pub(crate) fn le32(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4)
            .context("truncated archive field")?
            .try_into()?,
    ))
}
