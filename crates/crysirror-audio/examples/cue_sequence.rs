// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Independent cue timing + actual PCM oracle. No VM, game or audio device.
use anyhow::{Result, ensure};
use crysirror_audio::{
    decoder::Clip,
    mixer::{Mixer, MixerConfig, VoiceSpec},
    render_job::MixFrame,
    sequence::{Line, Sequences, Spec},
};
use crysirror_tasks::TaskPool;
use std::time::{Duration, Instant};
fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let path = args
        .first()
        .map(String::as_str)
        .unwrap_or("/tmp/crysirror-cue-sequence.wav");
    let worker = args.iter().any(|s| s == "--worker");
    let rate = 8000u32;
    let mut mixer = Mixer::new(MixerConfig {
        sample_rate: rate,
        ..Default::default()
    })?;
    let clips = [
        Clip::new(rate, vec![[0.25; 2]; 24])?,
        Clip::new(rate, vec![[-0.5; 2]; 16])?,
    ];
    let mut bank = Sequences::default();
    let h = bank.create(
        1,
        Spec {
            include_duration: true,
            lines: vec![
                Line {
                    key: "positive".into(),
                    weight: 1,
                    min_ms: 4,
                    max_ms: 4,
                },
                Line {
                    key: "negative".into(),
                    weight: 1,
                    min_ms: 4,
                    max_ms: 4,
                },
            ],
            ..Default::default()
        },
        1,
    )?;
    let pool = TaskPool::new("cue-demo", 1, 4)?;
    let mut pcm = vec![];
    let mut emissions = vec![];
    let mut completion = None;
    for ms in 0..20u64 {
        if ms > 0 {
            bank.advance(1, h, 1, 1)?;
        }
        while let Some(e) = bank.poll(1, h, None)? {
            emissions.push((ms, e.index));
            let clip = &clips[e.index];
            mixer.play(1, clip.source(), VoiceSpec::default())?;
            bank.complete(
                1,
                h,
                e.serial,
                (clip.frames() as u64 * 1000) / u64::from(rate),
            )?;
        }
        if !bank.info(1, h)?.active && completion.is_none() {
            completion = Some(ms);
        }
        let block = if worker {
            let mut job = MixFrame::new(pool.clone(), vec![8], vec![])?;
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(p) = job.poll(&mut [&mut mixer])? {
                    break p;
                }
                ensure!(Instant::now() < deadline, "DSP timeout");
                std::thread::yield_now();
            }
        } else {
            let mut p = vec![[0.; 2]; 8];
            mixer.render(&mut p)?;
            p
        };
        pcm.extend(block);
    }
    ensure!(
        emissions == vec![(0, 0), (7, 1)] && completion == Some(13),
        "cue timing: {emissions:?}/{completion:?}"
    );
    for (i, p) in pcm.iter().enumerate() {
        let expected = if i < 24 {
            0.25
        } else if (56..72).contains(&i) {
            -0.5
        } else {
            0.
        };
        ensure!(
            p.iter().all(|x| (x - expected).abs() < 1e-6),
            "sample {i}: {p:?} expected {expected}"
        );
    }
    let size = (pcm.len() * 4) as u32;
    let mut b = b"RIFF".to_vec();
    b.extend((36 + size).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(rate.to_le_bytes());
    b.extend((rate * 4).to_le_bytes());
    b.extend(4u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(size.to_le_bytes());
    for s in pcm.iter().flatten() {
        b.extend(((s * 32767.).round() as i16).to_le_bytes());
    }
    std::fs::write(path, b)?;
    println!(
        "{path}: stereo {rate} Hz / {} frames; cues={emissions:?}, sequence_complete_ms={completion:?}, worker={worker}",
        pcm.len()
    );
    Ok(())
}
