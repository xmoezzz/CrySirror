// SPDX-License-Identifier: MIT
// Derived format layouts: GARbro ArcGPK.cs / ArcVPK.cs, morkt 2015.
use crate::u32at;
use anyhow::{ensure, Context, Result};
use crysirror_core::service::AssetReader;
use crysirror_resource::{
    read_all, subrange, Archive, ArchiveEntry, ArchiveFormat, DecodeContext, Probe,
};
use std::{collections::BTreeSet, sync::Arc};
pub struct Gpk;
pub struct Vpk;
struct Container {
    source: Arc<dyn AssetReader>,
    entries: Vec<ArchiveEntry>,
    ranges: Vec<(u64, u64)>,
}
impl Archive for Container {
    fn entries(&self) -> &[ArchiveEntry] {
        &self.entries
    }
    fn open(&self, id: u32, cx: &DecodeContext) -> Result<Arc<dyn AssetReader>> {
        cx.check_cancelled()?;
        let &(at, len) = self
            .ranges
            .get(id as usize)
            .context("BlackCyc archive entry id")?;
        ensure!(len <= cx.limits.max_entry_size, "BlackCyc entry byte limit");
        subrange(self.source.clone(), at, len)
    }
}
fn name(raw: &[u8], extension: &str) -> Result<String> {
    let (name, bad) = encoding_rs::SHIFT_JIS.decode_without_bom_handling(raw);
    ensure!(
        !bad && !name.is_empty()
            && !name.contains(['/', '\\', ':', '\0'])
            && name != "."
            && name != "..",
        "BlackCyc archive filename"
    );
    Ok(format!("{name}.{extension}"))
}
fn open(source: Arc<dyn AssetReader>, cx: &DecodeContext, voice: bool) -> Result<Box<dyn Archive>> {
    cx.check_cancelled()?;
    let source_len = source.len().map_err(|e| anyhow::anyhow!("{e}"))?;
    ensure!(
        source_len < 0x80000000,
        "BlackCyc 64-bit/over-2GiB archive profile unsupported"
    );
    let logical = cx
        .source_name
        .as_deref()
        .context("BlackCyc archive needs source_name")?;
    let (stem, ext) = logical
        .rsplit_once('.')
        .context("BlackCyc archive extension absent")?;
    ensure!(
        ext.eq_ignore_ascii_case(if voice { "vpk" } else { "gpk" }),
        "BlackCyc archive extension/profile"
    );
    let sibling = format!(
        "{}.{}",
        stem.rsplit('/').next().unwrap(),
        if voice { "vtb" } else { "gtb" }
    );
    let table_source = cx.open_sibling(&sibling)?;
    let table = read_all(table_source.as_ref(), cx.limits.max_index_size)?;
    if !voice {
        let directory = crate::GpkIndex::parse(&table, cx)?;
        let mut entries = Vec::with_capacity(directory.entries().len());
        let mut ranges = Vec::with_capacity(directory.entries().len());
        for (id, entry) in directory.entries().iter().enumerate() {
            cx.check_cancelled()?;
            let end = entry.next_offset.unwrap_or(source_len);
            ensure!(
                entry.offset <= end
                    && end <= source_len
                    && end - entry.offset <= cx.limits.max_entry_size,
                "BlackCyc GPK entry bounds"
            );
            entries.push(ArchiveEntry {
                id: id as u32,
                path: entry.path.clone(),
                size: end - entry.offset,
                stored_size: end - entry.offset,
                metadata: serde_json::json!({"format":"black-cyc/gpk"}),
            });
            ranges.push((entry.offset, end - entry.offset));
        }
        return Ok(Box::new(Container {
            source,
            entries,
            ranges,
        }));
    }
    ensure!(
        table.len() >= 24 && table.len() % 12 == 0,
        "BlackCyc VTB record span"
    );
    let count = table.len() / 12 - 1;
    ensure!(
        count > 0 && count <= cx.limits.max_entries,
        "BlackCyc archive entry count"
    );
    let mut entries = Vec::with_capacity(count);
    let mut ranges = Vec::with_capacity(count);
    let mut seen = BTreeSet::new();
    let mut name_bytes = 0u64;
    for i in 0..count {
        cx.check_cancelled()?;
        let raw = &table[i * 12..i * 12 + 8];
        let len = raw.iter().position(|b| *b == 0).unwrap_or(8);
        let (path, at, end) = (
            name(&raw[..len], "vaw")?,
            u64::from(u32at(&table, i * 12 + 8)?),
            u64::from(u32at(&table, (i + 1) * 12 + 8)?),
        );
        ensure!(
            at <= end && end <= source_len && end - at <= cx.limits.max_entry_size,
            "BlackCyc entry placement/size"
        );
        name_bytes = name_bytes
            .checked_add(path.len() as u64)
            .context("BlackCyc directory name size overflow")?;
        ensure!(
            name_bytes <= cx.limits.max_index_size,
            "BlackCyc expanded directory name budget"
        );
        ensure!(
            seen.insert(path.to_ascii_lowercase()),
            "BlackCyc duplicate filename"
        );
        entries.push(ArchiveEntry {
            id: i as u32,
            path,
            size: end - at,
            stored_size: end - at,
            metadata: serde_json::json!({"format":"black-cyc/vpk"}),
        });
        ranges.push((at, end - at));
    }
    Ok(Box::new(Container {
        source,
        entries,
        ranges,
    }))
}
impl ArchiveFormat for Gpk {
    fn id(&self) -> &'static str {
        "black-cyc/gpk"
    }
    // These signatureless containers require their scoped companion directory;
    // raw body magic cannot identify an archive without that context.
    fn probe(&self, _: &dyn AssetReader) -> Result<Probe> {
        Ok(Probe::NO)
    }
    fn open(&self, s: Arc<dyn AssetReader>, cx: &DecodeContext) -> Result<Box<dyn Archive>> {
        open(s, cx, false)
    }
}
impl ArchiveFormat for Vpk {
    fn id(&self) -> &'static str {
        "black-cyc/vpk"
    }
    fn probe(&self, _: &dyn AssetReader) -> Result<Probe> {
        Ok(Probe::NO)
    }
    fn open(&self, s: Arc<dyn AssetReader>, cx: &DecodeContext) -> Result<Box<dyn Archive>> {
        open(s, cx, true)
    }
}
