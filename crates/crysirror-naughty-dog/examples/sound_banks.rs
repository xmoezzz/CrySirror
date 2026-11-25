// SPDX-License-Identifier: MPL-2.0
//! Index extracted Scream banks and resolve script sound ids through
//! `audio.bin`: per id its Scream name, bank, tones and the first tone's
//! decoded length.
//!
//! `sound_banks AUDIO_BIN BANK_DIR [ID…]`
use anyhow::{Context, Result};
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::sound::{Bank, Banks, SAMPLE_RATE, SoundTable};
use std::sync::Arc;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let table = SoundTable::parse(&std::fs::read(args.first().context("AUDIO_BIN")?)?)?;
    let dir = args.get(1).context("BANK_DIR")?;
    let mut banks = Banks::default();
    let (mut failed, mut tones) = (0, 0);
    for e in std::fs::read_dir(dir)? {
        let path = e?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        if !name.ends_with(".bnk") {
            continue;
        }
        match Bank::read(&name, Arc::new(MemoryAssetReader(std::fs::read(&path)?.into()))) {
            Ok(b) => {
                tones += b.sounds.values().map(|s| s.tones.len()).sum::<usize>();
                banks.add(b);
            }
            Err(e) => {
                failed += 1;
                println!("{name}: {e:#}");
            }
        }
    }
    let resolved = table.names.values().filter(|n| banks.sound(n).is_some()).count();
    let playable = table.names.values().filter(|n| banks.playable(n).is_some()).count();
    println!("{playable} records lead to a tone");
    println!("{} banks ({failed} failed), {tones} tones; {} sound records, {resolved} in a bank", banks.banks.len(), table.names.len());
    for id in &args[2..] {
        let id = u32::from_str_radix(id.trim_start_matches("0x"), 16)?;
        let Some(name) = table.names.get(&id) else {
            println!("{id:08x}: no record");
            continue;
        };
        match banks.playable(name).or(banks.sound(name)) {
            Some((bank, s)) if s.tones.is_empty() => println!("{id:08x} {name}: {} without tones", bank.name),
            Some((bank, s)) => {
                let (data, looped) = bank.wave(s.tones[0])?;
                let what = if data.starts_with(b"RIFF") { format!("{:.2} s PCM", (data.len() - 44) as f64 / 2. / f64::from(SAMPLE_RATE)) } else { format!("{} bytes MP3", data.len()) };
                println!("{id:08x} {name}: {} volume {} tones {} first {what} loop {looped:?}", bank.name, s.volume, s.tones.len());
            }
            None => println!("{id:08x} {name}: in no bank"),
        }
    }
    Ok(())
}
