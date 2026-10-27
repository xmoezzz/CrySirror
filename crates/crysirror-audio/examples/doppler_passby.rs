// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Authored moving oscillator and sample/frequency oracles; no VM or device.
use anyhow::{Result, ensure};
use crysirror_audio::{
    decoder::Clip,
    doppler::DopplerSettings,
    mixer::{Mixer, Spatial, VoiceSpec},
    render_job::MixFrame,
};
use crysirror_tasks::TaskPool;
use std::time::{Duration, Instant};
fn frequency(pcm: &[[f32; 2]], rate: u32) -> f64 {
    let crossings = pcm
        .windows(2)
        .filter(|p| p[0][0] + p[0][1] <= 0. && p[1][0] + p[1][1] > 0.)
        .count();
    crossings as f64 * f64::from(rate) / pcm.len() as f64
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let path = args
        .first()
        .map(String::as_str)
        .unwrap_or("/tmp/crysirror-doppler.wav");
    let worker = args.iter().any(|a| a == "--worker");
    let rate = 48000u32;
    let mut mixer = Mixer::default();
    let source = Clip::new(
        rate,
        (0..rate)
            .map(|i| [(std::f32::consts::TAU * 440. * i as f32 / rate as f32).sin() * 0.2; 2])
            .collect(),
    )?;
    let mut spatial = Spatial {
        position: [-40., 0., -4.],
        velocity: Some([40., 0., 0.]),
        doppler: Some(DopplerSettings::default()),
        min_distance: 100.,
        max_distance: 1000.,
        ..Default::default()
    };
    let h = mixer.play(
        1,
        source.source(),
        VoiceSpec {
            looping: true,
            spatial: Some(spatial.clone()),
            ..Default::default()
        },
    )?;
    let pool = TaskPool::new("Doppler-demo", 1, 4)?;
    let mut pcm = Vec::with_capacity(96000);
    for i in 0..200 {
        spatial.position = [-40. + 0.4 * i as f32, 0., -4.];
        mixer.set_spatial(1, h, Some(spatial.clone()))?;
        let block = if worker {
            let mut frame = MixFrame::new(pool.clone(), vec![480], vec![])?;
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(p) = frame.poll(&mut [&mut mixer])? {
                    break p;
                }
                ensure!(Instant::now() < deadline, "worker timed out");
                std::thread::yield_now();
            }
        } else {
            let mut p = vec![[0.; 2]; 480];
            mixer.render(&mut p)?;
            p
        };
        pcm.extend(block);
    }
    let approach = frequency(&pcm[9600..19200], rate);
    let recede = frequency(&pcm[76800..86400], rate);
    ensure!(
        approach > 475. && recede < 410.,
        "pass-by frequency oracle: {approach}/{recede}"
    );
    ensure!(
        mixer.spec(1, h)?.rate == 1. && mixer.spec(1, h)?.pitch_semitones == 0.,
        "Doppler rewrote user controls"
    );
    ensure!(
        pcm.iter().flatten().all(|s| s.is_finite() && s.abs() <= 1.),
        "nonfinite/clipped demo PCM"
    );
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
        "{path}: stereo {rate} Hz / {} frames; approach={approach:.2} Hz, recede={recede:.2} Hz, input=440 Hz, worker={worker}",
        pcm.len()
    );
    Ok(())
}
