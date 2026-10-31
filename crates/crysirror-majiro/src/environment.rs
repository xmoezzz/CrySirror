// Copyright (c) 2026 CrySirror contributors. MIT.
//! Original ENV tokens. This is native naming/configuration policy, not registry IO.
use anyhow::{ensure, Context, Result};
use std::collections::BTreeMap;
pub fn parse(bytes: &[u8]) -> Result<BTreeMap<String, String>> {
    ensure!(bytes.len() <= 256 * 1024, "Majiro ENV byte budget");
    let text = encoding_rs::SHIFT_JIS
        .decode_without_bom_handling_and_without_replacement(bytes)
        .context("invalid Majiro ENV Shift-JIS")?;
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let comment = line
            .match_indices("//")
            .find(|(i, _)| *i == 0 || line.as_bytes()[i - 1].is_ascii_whitespace())
            .map(|(i, _)| i)
            .unwrap_or(line.len());
        let line = line[..comment].trim();
        if line.is_empty() {
            continue;
        }
        if line == "[EOF]" {
            break;
        }
        let mut fields = line.split_whitespace();
        let key = fields.next().unwrap();
        let value = fields.next().context("Majiro ENV missing value")?;
        ensure!(
            key.is_ascii() && key.len() <= 64 && out.len() < 4096,
            "Majiro ENV token budget"
        );
        ensure!(
            out.insert(key.to_owned(), value.replace('|', " "))
                .is_none(),
            "duplicate Majiro ENV key"
        );
    }
    Ok(out)
}
