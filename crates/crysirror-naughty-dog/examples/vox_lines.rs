// SPDX-License-Identifier: MPL-2.0
//! Resolve the `vox.bin` dialogue table: per vox id the line `say`
//! plays, and the decoded duration of each line present in a directory of
//! extracted `<line>.xvag` files.
//!
//! `vox_lines VOX_BIN [LINE_DIR]`
use anyhow::{Context, Result};
use crysirror_naughty_dog::vox::VoxTable;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let table = VoxTable::parse(&std::fs::read(args.first().context("VOX_BIN")?)?)?;
    let lines: usize = table.vox.values().flat_map(|v| &v.tracks).map(|t| t.lines.len()).sum();
    println!("{} vox records, {} tracks, {lines} lines", table.vox.len(), table.vox.values().map(|v| v.tracks.len()).sum::<usize>());
    let mut ids: Vec<_> = table.vox.keys().copied().collect();
    ids.sort_unstable();
    let (mut decoded, mut seconds) = (0, 0.);
    for id in ids {
        let Some(line) = table.line(id, &[], 0) else { continue };
        let Some(dir) = args.get(1) else { continue };
        let Ok(bytes) = std::fs::read(format!("{dir}/{line}.xvag")) else { continue };
        let h = crysirror_audio::xvag::Header::parse(&bytes)?;
        let wave = crysirror_audio::xvag::to_wave(&bytes)?;
        let s = h.frames as f64 / f64::from(h.rate);
        println!("{id:08x} {line} {} ch {} Hz {s:.2} s wave {} bytes", h.channels, h.rate, wave.len());
        decoded += 1;
        seconds += s;
    }
    println!("decoded {decoded} lines, {seconds:.1} s");
    Ok(())
}
