// SPDX-License-Identifier: MPL-2.0
//! Print the HUD elements of an extracted `hud-settings.bin`.
use anyhow::{Context, Result};

fn main() -> Result<()> {
    let path = std::env::args().nth(1).context("HUD_SETTINGS_BIN")?;
    let defs = crysirror_naughty_dog::hud::parse(&std::fs::read(path)?)?;
    let mut v: Vec<_> = defs.iter().collect();
    v.sort_by_key(|(k, _)| **k);
    for (k, d) in v {
        println!("{k:08x} {:?} text {:?} images {:?}", d.rect, d.text, d.images);
    }
    Ok(())
}
