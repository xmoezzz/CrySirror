// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Sound Loop Information (`.sli`, KiriKiri WaveLoopManager text format
//! "#2.00"): links (jumps between sample positions, optionally conditioned on
//! integer flags) and named labels. Parsing only; playback consumes the result.
use anyhow::{Context, Result, bail, ensure};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Condition {
    /// Always taken.
    None,
    Equal,
    NotEqual,
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub from: u64,
    pub to: u64,
    pub smooth: bool,
    pub condition: Condition,
    pub ref_value: i64,
    pub cond_var: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub position: u64,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoopInfo {
    pub links: Vec<Link>,
    pub labels: Vec<Label>,
}

/// Flag slots addressable by `CondVar` (WaveLoopManager MaxFlags).
pub const MAX_FLAGS: usize = 16;

impl Link {
    /// Whether the link is taken with these flag values.
    pub fn taken(&self, flags: &[i64; MAX_FLAGS]) -> bool {
        let v = flags[self.cond_var];
        match self.condition {
            Condition::None => true,
            Condition::Equal => v == self.ref_value,
            Condition::NotEqual => v != self.ref_value,
            Condition::Greater => v > self.ref_value,
            Condition::GreaterOrEqual => v >= self.ref_value,
            Condition::Less => v < self.ref_value,
            Condition::LessOrEqual => v <= self.ref_value,
        }
    }
}

pub fn parse(text: &str) -> Result<LoopInfo> {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines();
    let header = lines.next().unwrap_or_default().trim();
    ensure!(header == "#2.00", "unsupported sound loop information version `{header}`");
    let mut info = LoopInfo::default();
    for line in lines {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (kind, body) = line.split_once('{').context("sli entry without `{`")?;
        let body = body.trim().strip_suffix('}').context("sli entry without `}`")?;
        let mut fields = std::collections::BTreeMap::new();
        for item in body.split(';') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            let (k, v) = item.split_once('=').context("sli field without `=`")?;
            fields.insert(k.trim().to_owned(), v.trim().to_owned());
        }
        let num = |k: &str| -> Result<i64> {
            fields.get(k).with_context(|| format!("sli field {k} missing"))?.parse().with_context(|| format!("sli field {k}"))
        };
        let pos = |k: &str| -> Result<u64> { u64::try_from(num(k)?).with_context(|| format!("sli field {k} negative")) };
        match kind.trim() {
            "Link" => {
                let condition = match fields.get("Condition").map(String::as_str).unwrap_or("no") {
                    "no" => Condition::None,
                    "eq" => Condition::Equal,
                    "ne" => Condition::NotEqual,
                    "gt" => Condition::Greater,
                    "ge" => Condition::GreaterOrEqual,
                    "lt" => Condition::Less,
                    "le" => Condition::LessOrEqual,
                    other => bail!("unknown sli link condition `{other}`"),
                };
                let cond_var = usize::try_from(num("CondVar")?).ok().filter(|v| *v < MAX_FLAGS).context("sli CondVar range")?;
                let smooth = match fields.get("Smooth").map(String::as_str) {
                    Some("True") => true,
                    Some("False") | None => false,
                    Some(other) => bail!("sli Smooth value `{other}`"),
                };
                info.links.push(Link {
                    from: pos("From")?,
                    to: pos("To")?,
                    smooth,
                    condition,
                    ref_value: num("RefValue")?,
                    cond_var,
                });
            }
            "Label" => {
                let name = fields.get("Name").context("sli label name missing")?;
                let name = name
                    .strip_prefix('\'')
                    .and_then(|n| n.strip_suffix('\''))
                    .context("sli label name must be quoted")?;
                info.labels.push(Label { position: pos("Position")?, name: name.to_owned() });
            }
            other => bail!("unknown sli entry `{other}`"),
        }
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_links_and_labels() {
        let info = parse(
            "#2.00\n# Sound Loop Information (utf-8)\n\
             Link { From=11148424;         To=749200;           Smooth=False; Condition=eq; RefValue=0;          CondVar=0;  }\n\
             Label { Position=6326784;          Name='sabi';                                      }\n",
        )
        .unwrap();
        assert_eq!(info.links.len(), 1);
        let l = &info.links[0];
        assert_eq!((l.from, l.to, l.smooth, l.condition), (11148424, 749200, false, Condition::Equal));
        assert!(l.taken(&[0; MAX_FLAGS]));
        let mut flags = [0; MAX_FLAGS];
        flags[0] = 1;
        assert!(!l.taken(&flags));
        assert_eq!(info.labels, vec![Label { position: 6326784, name: "sabi".into() }]);
        assert!(parse("#1.00\n").is_err());
    }
}
