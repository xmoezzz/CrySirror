// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Authored offline compression demo; no VM, game assets or output device.
use anyhow::{Result, ensure};
use crysirror_audio::voice_compression::siglus_jitan_cancellable;
use crysirror_resource::WaveSpec;
use crysirror_tasks::TaskPool;
fn main() -> Result<()> {
    let directory = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .unwrap_or_else(|| "/tmp/crysirror-voice-compression".into()),
    );
    std::fs::create_dir_all(&directory)?;
    let rate = 48000;
    let input = (0..rate)
        .map(|i| {
            let t = i as f64 / rate as f64;
            ((t * std::f64::consts::TAU * 440.).sin() * 9000.
                + (t * std::f64::consts::TAU * 880.).sin() * 3000.) as i16
        })
        .collect::<Vec<_>>();
    let spec = WaveSpec {
        sample_rate: rate,
        ..Default::default()
    };
    std::fs::write(
        directory.join("original.wav"),
        spec.encode(
            &input
                .iter()
                .flat_map(|s| s.to_le_bytes())
                .collect::<Vec<_>>(),
        )?,
    )?;
    let pool = TaskPool::new("demo-compression", 1, 4)?;
    let result = pool
        .spawn(move |cancel| siglus_jitan_cancellable(&input, 1, rate, 200, &cancel))?
        .wait()?;
    ensure!(
        result.rate == rate && result.channels == 1 && result.samples.len() < rate as usize,
        "compression contract"
    );
    let output_frames = result.samples.len();
    std::fs::write(
        directory.join("compressed.wav"),
        spec.encode(
            &result
                .samples
                .iter()
                .flat_map(|s| s.to_le_bytes())
                .collect::<Vec<_>>(),
        )?,
    )?;
    println!(
        "{}",
        serde_json::json!({"source_rate":rate,"input_frames":rate,"output_frames":output_frames,"percent":200,"device":false,"native_vm":false,"directory":directory})
    );
    Ok(())
}
