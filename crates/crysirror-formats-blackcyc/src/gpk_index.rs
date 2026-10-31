// BlackCyc GTB format facts derived from GARbro, morkt 2015. MIT.
// See LICENSE and SOURCE.json.
//! Detached companion directory. Native precedence can inspect a directory
//! before opening its body, without duplicating a parser or inventing a body.
use crate::u32at;
use anyhow::{ensure, Context, Result};
use crysirror_resource::DecodeContext;
use std::collections::BTreeSet;
#[derive(Clone, Debug)]
pub struct GpkIndexEntry {
    pub path: String,
    pub offset: u64,
    pub next_offset: Option<u64>,
}
#[derive(Clone, Debug)]
pub struct GpkIndex {
    entries: Vec<GpkIndexEntry>,
}
impl GpkIndex {
    pub fn parse(table: &[u8], cx: &DecodeContext) -> Result<Self> {
        cx.check_cancelled()?;
        ensure!(
            table.len() as u64 <= cx.limits.max_index_size,
            "BlackCyc GTB byte budget"
        );
        let count = u32at(table, 0)? as usize;
        ensure!(
            count > 0 && count <= cx.limits.max_entries,
            "BlackCyc GTB entry count"
        );
        let names = 4usize
            .checked_add(count.checked_mul(8).context("GTB arrays overflow")?)
            .context("GTB arrays overflow")?;
        ensure!(names < table.len(), "BlackCyc GTB arrays");
        let mut entries = Vec::with_capacity(count);
        let mut seen = BTreeSet::new();
        let mut name_bytes = 0usize;
        for index in 0..count {
            cx.check_cancelled()?;
            let at = names
                .checked_add(u32at(table, 4 + index * 4)? as usize)
                .context("GTB name offset overflow")?;
            let tail = table.get(at..).context("GTB name outside directory")?;
            let len = tail
                .iter()
                .take(1025)
                .position(|b| *b == 0)
                .context("GTB name missing terminator")?;
            let (name, bad) = encoding_rs::SHIFT_JIS.decode_without_bom_handling(&tail[..len]);
            ensure!(
                !bad && !name.is_empty()
                    && !name.contains(['/', '\\', ':', '\0'])
                    && name != "."
                    && name != "..",
                "BlackCyc GTB basename"
            );
            let path = format!("{name}.dwq");
            name_bytes = name_bytes
                .checked_add(path.len())
                .context("GTB expanded names overflow")?;
            ensure!(
                name_bytes as u64 <= cx.limits.max_index_size,
                "BlackCyc GTB expanded name budget"
            );
            ensure!(
                seen.insert(path.to_ascii_lowercase()),
                "BlackCyc GTB duplicate basename"
            );
            let offset = u64::from(u32at(table, 4 + count * 4 + index * 4)?);
            let next_offset = if index + 1 == count {
                None
            } else {
                Some(u64::from(u32at(table, 4 + count * 4 + (index + 1) * 4)?))
            };
            ensure!(
                next_offset.is_none_or(|next| next >= offset),
                "BlackCyc GTB unordered offsets"
            );
            entries.push(GpkIndexEntry {
                path,
                offset,
                next_offset,
            });
        }
        Ok(Self { entries })
    }
    pub fn entries(&self) -> &[GpkIndexEntry] {
        &self.entries
    }
    pub fn find(&self, path: &str) -> Option<&GpkIndexEntry> {
        self.entries
            .iter()
            .find(|e| e.path.eq_ignore_ascii_case(path))
    }
}
