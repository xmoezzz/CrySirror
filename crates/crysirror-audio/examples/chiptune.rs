// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Authored OPNA register score -> shared source -> worker Mixer -> stereo WAVE.
use anyhow::{ensure, Result};
use crysirror_audio::{
    mixer::{Bus, Mixer, MixerConfig, VoiceSpec},
    render_job::MixFrame,
    synth::{RegisterScore, RegisterWrite, SynthSource},
};
fn main() -> Result<()> {
    let out = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "/tmp/crysirror-chiptune".into()),
    );
    std::fs::create_dir_all(&out)?;
    let rate = 48000;
    let mut writes = vec![];
    let mut write = |frame, register, value| {
        writes.push(RegisterWrite {
            frame,
            port: 0,
            register,
            value,
        })
    };
    write(0, 0xb0, 7);
    write(0, 0xb4, 0xc0);
    for r in [0x30, 0x34, 0x38, 0x3c] {
        write(0, r, 1);
        write(0, r + 0x10, 127);
        write(0, r + 0x20, 31);
        write(0, r + 0x50, 15);
    }
    write(0, 0x4c, 0);
    write(0, 0x11, 63);
    write(0, 0x18, 0xdf);
    let notes = [(1040, 4), (1237, 4), (1558, 4), (1040, 5)];
    for i in 0..8u64 {
        let at = i * 12000;
        let (fnum, block) = notes[i as usize % 4];
        write(at, 0xa4, (block << 3) | ((fnum >> 8) as u8 & 7));
        write(at, 0xa0, fnum as u8);
        write(at, 0x28, 0x80);
        if i % 2 == 0 {
            write(at, 0x10, 1);
        }
        write(at + 10500, 0x28, 0);
    }
    let score = RegisterScore {
        schema: 1,
        output_rate: rate,
        frames: 96000,
        writes,
    };
    score.validate()?;
    std::fs::write(out.join("lead.opna"), serde_json::to_vec_pretty(&score)?)?;
    let mut mixer = Mixer::new(MixerConfig {
        sample_rate: rate,
        ..Default::default()
    })?;
    mixer.play(
        1,
        Box::new(SynthSource::registers(score)?),
        VoiceSpec {
            bus: Bus::Music,
            gain: 0.5,
            ..Default::default()
        },
    )?;
    let pool = crysirror_tasks::TaskPool::new("synth-demo", 1, 4)?;
    let mut pcm = vec![];
    while pcm.len() < 96000 {
        let n = (96000 - pcm.len()).min(4096);
        let mut job = MixFrame::new(pool.clone(), vec![n], vec![])?;
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Some(block) = job.poll(&mut [&mut mixer])? {
                pcm.extend(block);
                break;
            }
            ensure!(std::time::Instant::now() < end, "synth DSP deadline");
            std::thread::yield_now();
        }
    }
    ensure!(
        pcm.iter().flatten().all(|v| v.is_finite()),
        "nonfinite synth output"
    );
    let peak = pcm.iter().flatten().map(|v| v.abs()).fold(0f32, f32::max);
    ensure!(peak > 0.01, "silent synth output");
    let size = (pcm.len() * 4) as u32;
    let mut wav = b"RIFF".to_vec();
    wav.extend((36 + size).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(rate.to_le_bytes());
    wav.extend((rate * 4).to_le_bytes());
    wav.extend(4u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(size.to_le_bytes());
    for v in pcm.iter().flatten() {
        wav.extend(((v.clamp(-1., 1.) * 32767.).round() as i16).to_le_bytes());
    }
    std::fs::write(out.join("lead.wav"), wav)?;
    println!(
        "{}: 96000 stereo frames, 48000 Hz, worker DSP, peak={peak:.4}",
        out.display()
    );
    Ok(())
}
