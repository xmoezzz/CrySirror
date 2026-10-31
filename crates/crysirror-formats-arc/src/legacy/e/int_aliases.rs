// SPDX-License-Identifier: GPL-3.0-or-later
//! An INT namespace may contain byte-identical repeated native names. We only
//! coalesce proven equivalents; conflicting native winner policy is unsupported.
use super::*;
use crysirror_core::path::{Mount, VfsPath};
use std::collections::BTreeMap;
const MAX_COMPARE_BYTES: u64 = 64 * 1024 * 1024;
pub(super) fn coalesce(
    source: &dyn AssetReader,
    cx: &DecodeContext,
    rows: Vec<Row>,
    known_decode: bool,
) -> Result<Vec<Row>> {
    let rows = checked_rows(source, cx, rows)?;
    let mut output = Vec::<Row>::new();
    let mut names = BTreeMap::<String, usize>::new();
    let mut compared = 0u64;
    for (original_id, mut row) in rows.into_iter().enumerate() {
        cx.check_cancelled()?;
        let path = VfsPath::new(Mount::Asset, &row.path.replace('\\', "/"))?
            .relative()
            .to_owned();
        let provenance = serde_json::json!({"entry_id":original_id,"native_index":row.meta.get("native_index"),"path":row.path,"offset":row.offset,"stored_size":row.stored,"size":row.size});
        if let Some(&index) = names.get(&path) {
            let prior = &mut output[index];
            ensure!(
                known_decode,
                "unsupported CatSystem2 INT duplicate path with custom transform: {path}"
            );
            ensure!(
                row.stored == prior.stored && row.size == prior.size,
                "unsupported CatSystem2 INT conflicting duplicate path: {path}"
            );
            if row.offset != prior.offset {
                compared = compared
                    .checked_add(
                        row.stored
                            .checked_mul(2)
                            .context("INT alias compare extent overflow")?,
                    )
                    .context("INT alias byte overflow")?;
                ensure!(
                    compared <= MAX_COMPARE_BYTES,
                    "CatSystem2 INT alias comparison byte budget"
                );
                let mut at = 0;
                while at < row.stored {
                    cx.check_cancelled()?;
                    let count = (row.stored - at).min(65536);
                    let first = bytes(source, prior.offset + at, count, 65536)?;
                    let second = bytes(source, row.offset + at, count, 65536)?;
                    ensure!(
                        first == second,
                        "unsupported CatSystem2 INT conflicting duplicate payload: {path}"
                    );
                    at += count;
                }
            }
            prior.meta["identical_native_rows"]
                .as_array_mut()
                .context("INT alias metadata invariant")?
                .push(provenance);
        } else {
            names.insert(path.clone(), output.len());
            row.path = path;
            if !row.meta.is_object() {
                row.meta = serde_json::json!({});
            }
            row.meta["identical_native_rows"] = serde_json::json!([provenance]);
            row.meta["namespace_policy"] =
                serde_json::json!("only_proven_identical_native_aliases");
            output.push(row);
        }
    }
    Ok(output)
}
