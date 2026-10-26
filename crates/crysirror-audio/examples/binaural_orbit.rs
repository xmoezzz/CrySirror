// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Offline engine mixer + external HRIR demo. No native engine or output device.
//! Supply a licensed SOFA/JSON profile and output path. Nothing is bundled.
use crysirror_acoustics::binaural::{HrtfDataset, HrtfOptions};
use crysirror_audio::{
    decoder::Clip,
    mixer::{Mixer, Spatial, VoiceSpec},
};
use std::time::Instant;
fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    anyhow::ensure!(
        args.len() == 2,
        "usage: binaural_orbit <profile.sofa|json> <output.wav>"
    );
    let start = Instant::now();
    let dataset =
        HrtfDataset::from_bytes(&std::fs::read(&args[0])?, 48000, HrtfOptions::default())?;
    let preparation_ms = start.elapsed().as_secs_f64() * 1000.;
    let mut seed = 17u32;
    let clip = Clip::new(
        48000,
        (0..48000 * 8)
            .map(|i| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = ((seed >> 8) as f32 / 8388608. - 1.) * 0.05;
                let envelope = if i % 24000 < 2400 { 1. } else { 0.1 };
                let sample = envelope
                    * (noise + (std::f32::consts::TAU * 440. * i as f32 / 48000.).sin() * 0.05);
                [sample, sample]
            })
            .collect(),
    )?;
    let mut mixer = Mixer::new(Default::default())?;
    let h = mixer.play(
        1,
        clip.source(),
        VoiceSpec {
            spatial: Some(Spatial {
                position: [1., 0., 0.],
                min_distance: 10.,
                max_distance: 20.,
                ..Default::default()
            }),
            ..Default::default()
        },
    )?;
    let (mut publisher, voice) = dataset.voice([1., 0., 0.])?;
    let tail = voice.tail_frames();
    mixer.set_binaural(1, h, Some(voice))?;
    let mut samples = vec![];
    let mut dsp_ns = 0;
    let mut filters_ns = 0;
    for block in 0..800 {
        let angle = std::f32::consts::TAU * block as f32 / 800.;
        let position = [angle.cos(), 0.35 * (angle * 2.).sin(), angle.sin()];
        mixer.set_spatial(
            1,
            h,
            Some(Spatial {
                position,
                min_distance: 10.,
                max_distance: 20.,
                ..Default::default()
            }),
        )?;
        if block % 10 == 0 {
            let clock = Instant::now();
            publisher.publish(dataset.prepare_filter(position)?)?;
            filters_ns += clock.elapsed().as_nanos();
        }
        let mut output = [[0.; 2]; 480];
        let clock = Instant::now();
        mixer.render(&mut output)?;
        dsp_ns += clock.elapsed().as_nanos();
        samples.extend(output);
    }
    let mut output = vec![[0.; 2]; tail + 64];
    mixer.render(&mut output)?;
    samples.extend(output);
    publisher.reclaim();
    let rate = 48000u32;
    let size = (samples.len() * 4) as u32;
    let mut wav = Vec::with_capacity(size as usize + 44);
    wav.extend(b"RIFF");
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
    for sample in samples.into_iter().flatten() {
        wav.extend(((sample.clamp(-1., 1.) * 32767.).round() as i16).to_le_bytes());
    }
    std::fs::write(&args[1], wav)?;
    println!(
        "profile={} prepare_ms={preparation_ms:.3} render_8s_ms={:.3} filter_updates_ms={:.3} output={}",
        dataset.metadata(),
        dsp_ns as f64 / 1e6,
        filters_ns as f64 / 1e6,
        args[1]
    );
    Ok(())
}
