// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Authored transport sample oracle/export; no VM, game, or audio device.
use anyhow::{Result, ensure};
use crysirror_audio::{
    decoder,
    mixer::{Mixer, MixerConfig, VoiceSpec},
};
use crysirror_core::service::{AudioResumePolicy, PcmAudio};
fn render(m: &mut Mixer, out: &mut Vec<[f32; 2]>, n: usize) -> Result<()> {
    let start = out.len();
    out.resize(start + n, [0.; 2]);
    m.render(&mut out[start..])
}
fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/crysirror-transport.wav".into());
    let rate = 8000u32;
    let mut m = Mixer::new(MixerConfig {
        sample_rate: rate,
        ..Default::default()
    })?;
    let h = m.play(
        1,
        decoder::open_pcm(PcmAudio::new(rate, vec![[0.25; 2]; 1600])?),
        VoiceSpec::default(),
    )?;
    let mut out = vec![];
    render(&mut m, &mut out, 400)?;
    m.pause_fade(1, h, true, 50.)?;
    m.schedule_resume_with_policy(1, h, 100., 25., AudioResumePolicy::Preserve)?;
    render(&mut m, &mut out, 800)?;
    let info = m.playback_info(1, h)?;
    ensure!(
        info.age_frames == 1200 && info.transport_frames == 800 && info.position_frames == 800,
        "pause/delay transport clock oracle"
    );
    render(&mut m, &mut out, 400)?;
    m.stop(1, h, 100.)?;
    render(&mut m, &mut out, 400)?;
    let info = m.playback_info(1, h)?;
    ensure!(
        !info.source_playing && info.stopping && info.fade_out_frames == 400,
        "EOF shortened stop fade"
    );
    render(&mut m, &mut out, 400)?;
    ensure!(!m.is_playing(1, h), "stop deadline");
    for (i, p) in out.iter().enumerate() {
        let expected = match i {
            0..400 => 0.25,
            400..800 => 0.25 * (1. - (i - 400) as f32 / 400.),
            800..1200 => 0.,
            1200..1400 => 0.25 * (i - 1200) as f32 / 200.,
            1400..1600 => 0.25,
            1600..2000 => 0.25 * (1. - (i - 1600) as f32 / 800.),
            _ => 0.,
        };
        ensure!(
            (p[0] - expected).abs() < 1e-6 && p[0] == p[1],
            "sample {i} transport oracle"
        );
    }
    let size = (out.len() * 4) as u32;
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
    for sample in out.iter().flatten() {
        b.extend(((sample * 32767.).round().clamp(-32768., 32767.) as i16).to_le_bytes());
    }
    std::fs::write(&path, b)?;
    println!(
        "{path}: stereo {rate} Hz / {} frames; running, fade-pause, delayed resume, EOF and retained stop completion verified",
        out.len()
    );
    Ok(())
}
