// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Engine-owned hierarchical music/UI gain automation; no VM/device/game data.
use anyhow::{Result, ensure};
use crysirror_audio::{
    decoder,
    mixer::{Bus, Mixer, VoiceSpec},
};
use crysirror_core::service::{AudioMixGroupSettings as Settings, PcmAudio};
fn rms(s: &[[f32; 2]]) -> f64 {
    (s.iter().map(|s| (s[0] as f64).powi(2)).sum::<f64>() / s.len() as f64).sqrt()
}
fn main() -> Result<()> {
    let path = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .unwrap_or_else(|| "/tmp/crysirror-mix-groups.wav".into()),
    );
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let worker = std::env::args().any(|arg| arg == "--worker");
    let pool = worker
        .then(|| crysirror_tasks::TaskPool::new("demo-mix", 2, 4))
        .transpose()?;
    let rate = 48000u32;
    let mut mixer = Mixer::default();
    mixer.set_mix_group(1, "world", Settings::default(), 0.)?;
    mixer.set_mix_group(
        1,
        "music",
        Settings {
            parent: Some("world".into()),
            gain: 0.5,
            ..Default::default()
        },
        0.,
    )?;
    mixer.set_mix_group(
        1,
        "ui",
        Settings {
            gain: 0.5,
            ..Default::default()
        },
        0.,
    )?;
    let mut ui = Vec::new();
    for (name, bus, freq, amplitude) in
        [("music", Bus::Music, 220., 0.4), ("ui", Bus::Ui, 880., 0.2)]
    {
        let samples = (0..rate)
            .map(|i| [(std::f32::consts::TAU * freq * i as f32 / rate as f32).sin() * amplitude; 2])
            .collect::<Vec<_>>();
        if name == "ui" {
            ui = samples.clone();
        }
        mixer.play(
            1,
            decoder::open_pcm(PcmAudio::new(rate, samples)?),
            VoiceSpec {
                bus,
                mix_group: Some(name.into()),
                looping: true,
                ..Default::default()
            },
        )?;
    }
    let mut out = vec![[0.; 2]; rate as usize * 3];
    for second in 0..3 {
        if second > 0 {
            mixer.set_mix_group_gain(1, "world", if second == 1 { 0. } else { 1. }, 250.)?;
        }
        for block in out[second * rate as usize..(second + 1) * rate as usize].chunks_mut(512) {
            if let Some(pool) = &pool {
                let mut job = crysirror_audio::render_job::MixJob::begin(
                    pool,
                    &mut [&mut mixer],
                    block.len(),
                    vec![],
                )?;
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                loop {
                    if let Some(pcm) = job.poll(&mut [&mut mixer])? {
                        block.copy_from_slice(&pcm);
                        break;
                    }
                    ensure!(
                        std::time::Instant::now() < deadline,
                        "demo audio worker timeout"
                    );
                    std::thread::yield_now();
                }
            } else {
                mixer.render(block)?;
            }
        }
    }
    let n = rate as usize;
    // Once the parent is silent, UI still equals its independent source/group.
    for (i, s) in out[n + n / 2..n * 2].iter().enumerate() {
        ensure!(
            (s[0] - ui[n / 2 + i][0] * 0.5).abs() < 2e-6,
            "UI inherited an unrelated parent"
        );
    }
    let clear = rms(&out[n / 2..n]);
    let quiet = rms(&out[n + n / 2..n * 2]);
    let restored = rms(&out[n * 2 + n / 2..n * 3]);
    ensure!(
        (clear - restored).abs() < 1e-6 && quiet < clear * 0.5,
        "group automation oracle"
    );
    let peak = out.iter().flatten().fold(0f32, |a, s| a.max(s.abs()));
    ensure!(peak <= 1., "demo clips");
    let bytes = (out.len() * 4) as u32;
    let mut wave = b"RIFF".to_vec();
    wave.extend((bytes + 36).to_le_bytes());
    wave.extend(b"WAVEfmt ");
    wave.extend(16u32.to_le_bytes());
    wave.extend(1u16.to_le_bytes());
    wave.extend(2u16.to_le_bytes());
    wave.extend(rate.to_le_bytes());
    wave.extend((rate * 4).to_le_bytes());
    wave.extend(4u16.to_le_bytes());
    wave.extend(16u16.to_le_bytes());
    wave.extend(b"data");
    wave.extend(bytes.to_le_bytes());
    for s in out.iter().flatten() {
        wave.extend(((*s * 32767.).round() as i16).to_le_bytes());
    }
    std::fs::write(&path, wave)?;
    println!(
        "{}: stereo48k, three seconds; parent fades music 1–1.25s and restores 2–2.25s; UI unaffected; worker={worker}; RMS={clear:.6}/{quiet:.6}/{restored:.6}; peak={peak:.6}",
        path.display()
    );
    Ok(())
}
