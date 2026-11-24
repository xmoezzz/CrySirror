// SPDX-License-Identifier: MPL-2.0
//! Opt-in static parser coverage. Never execute commands or print RLD text.
use crate::{Script, branch::number, cstring, word};
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, fs, path::PathBuf};

fn mode(op: u16, words: &[i32], strings: &[&[u8]]) -> String {
    let ws: &[usize] = match op {
        0x07 => &[0, 2, 3, 5],
        0x0d | 0x33 | 0x4e | 0x5f | 0x65 | 0x6a => &[1],
        0x12 | 0x13 => &[0],
        0x32 => &[4, 5],
        0x48 => &[0],
        0x38..=0x3a => &[2, 3],
        0x4a => &[1, 2, 13],
        0x51 => &[5, 6, 7],
        0x52 => &[0, 1],
        0x5c => &[9],
        0x71 | 0x8b => &[2, 3],
        0x72 => &[0],
        0x75 => &[0, 2],
        0x76 => &[0, 2, 7, 8],
        0x77 => &[0, 1, 3],
        _ => &[],
    };
    let fields: Vec<_> = strings
        .first()
        .copied()
        .unwrap_or_default()
        .splitn(34, |&b| b == b',')
        .collect();
    let sub = fields.first().and_then(|f| number(f).ok()).unwrap_or(-1);
    let cs: &[usize] = match op {
        0x06 | 0x08 | 0x0a => &[0, 2],
        0x0b if matches!(sub, 31 | 39 | 42) => &[0, 2, 3],
        0x0b => &[0, 2],
        0x2c => &[0],
        0x46 => &[2],
        0x49 => &[1],
        0x5b => &[1, 7, 9],
        0x5e => &[0, 9],
        0x64 => &[2, 3],
        0x69 => &[1],
        _ => &[],
    };
    let parts: Vec<_> = ws
        .iter()
        .filter_map(|&i| words.get(i).map(|n| format!("w{i}:{n}")))
        .chain(cs.iter().filter_map(|&i| {
            fields
                .get(i)
                .and_then(|f| number(f).ok())
                .map(|n| format!("s{i}:{n}"))
        }))
        .collect();
    if parts.is_empty() {
        "?".into()
    } else {
        parts.join("/")
    }
}

#[test]
#[ignore = "requires an explicitly supplied loose v1 installation; static parser only"]
fn static_v1_corpus_census() -> Result<()> {
    let root = PathBuf::from(
        std::env::var_os("CRYSIRROR_EXHIBIT_CENSUS_ROOT")
            .context("set CRYSIRROR_EXHIBIT_CENSUS_ROOT to the local installed game")?,
    );
    let catalog =
        crate::message_area::Catalog::from_v1_definition(fs::read(root.join("rld/def.rld"))?)?;
    println!("V1_MESSAGE_AREAS ids={:?}", catalog.audit_ids());
    let controls =
        crate::local_controls::Catalog::from_v1_definition(fs::read(root.join("rld/def.rld"))?)?;
    println!("V1_LOCAL_CONTROLS ids={:?}", controls.audit_ids());
    let mut prefix_bytes = 0;
    let mut opcode_bytes = 0;
    let mut operand_bytes = 0;
    let mut string_bytes = 0;
    let mut summary = BTreeMap::<(u16, u8, usize, String), [usize; 2]>::new();
    let mut files = 0;
    let mut records = 0;
    for entry in fs::read_dir(root.join("rld"))? {
        let path = entry?.path();
        if !path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("rld"))
        {
            continue;
        }
        let mut script = Script::new(fs::read(path)?, 1)?;
        files += 1;
        let mut at = script.pc;
        prefix_bytes += at;
        for _ in 0..script.remaining {
            let pc = at;
            let h = word(&script.bytes, at)?;
            let op = h as u16;
            let flags = (h >> 24) as u8;
            let arity = ((h >> 16) & 255) as usize;
            at += 4;
            opcode_bytes += 4;
            operand_bytes += 4 * arity;
            let mut words = Vec::with_capacity(arity);
            for _ in 0..arity {
                words.push(word(&script.bytes, at)? as i32);
                at += 4;
            }
            let mut strings = Vec::new();
            for _ in 0..(flags & 15) {
                let (raw, end) = cstring(&script.bytes, at)?;
                strings.push(raw);
                string_bytes += end - at;
                at = end;
            }
            let signature = (op, flags, arity, mode(op, &words, &strings));
            script.pc = pc;
            script.remaining = 1;
            let accepted = script.next().is_ok();
            if accepted {
                ensure!(script.pc == at, "parser consumed partial opcode {op:#x}");
            }
            summary.entry(signature).or_default()[usize::from(accepted)] += 1;
            records += 1;
        }
        ensure!(
            at == script.bytes.len(),
            "record counts did not consume v1 file"
        );
    }
    println!(
        "V1_CENSUS files={files} records={records} signatures={}",
        summary.len()
    );
    println!(
        "V1_BYTES prefix={prefix_bytes} opcodes={opcode_bytes} numeric={operand_bytes} strings={string_bytes} leftovers=0"
    );
    for ((op, flags, arity, mode), [rejected, admitted]) in summary {
        println!(
            "V1_ROW {op:04x}\t{flags:02x}\t{arity}\t{}\t{mode}\t{admitted}\t{rejected}",
            flags & 15
        );
    }
    Ok(())
}
