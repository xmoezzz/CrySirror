// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Offline, synthesized-only PCM demo. First three seconds clear, next three
//! behind a frequency-selective wall, last three clear. No device or VM.
use crysirror_acoustics::{
    AcousticMaterial, AcousticMesh, AcousticQuery, AcousticScene, AcousticSceneSpec, AcousticSource,
};
use crysirror_audio::{
    decoder::Clip,
    mixer::{Mixer, MixerConfig, Spatial, VoiceSpec},
};
fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "acoustic-room.wav".into());
    let scene = AcousticScene::build(AcousticSceneSpec {
        materials: vec![AcousticMaterial {
            transmission: [0.7, 0.25, 0.04],
            ..Default::default()
        }],
        meshes: vec![AcousticMesh {
            id: 1,
            vertices: vec![
                [-10., -10., -2.],
                [10., -10., -2.],
                [10., 10., -2.],
                [-10., 10., -2.],
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            materials: vec![],
            transform: [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ],
        }],
        ..Default::default()
    })?;
    let rate = 48000;
    let clip = Clip::new(
        rate,
        (0..rate)
            .map(|i| {
                let t = i as f32 / rate as f32;
                let v = [200., 3000., 10000.]
                    .map(|f| (std::f32::consts::TAU * f * t).sin())
                    .iter()
                    .sum::<f32>()
                    * 0.14;
                [v, v]
            })
            .collect(),
    )?;
    let mut mixer = Mixer::new(MixerConfig {
        sample_rate: rate,
        ..Default::default()
    })?;
    let voice = mixer.play(
        1,
        clip.source(),
        VoiceSpec {
            looping: true,
            spatial: Some(Spatial {
                position: [0., 0., -4.],
                min_distance: 10.,
                max_distance: 20.,
                ..Default::default()
            }),
            ..Default::default()
        },
    )?;
    let path_result = crysirror_acoustics::simulate(
        &scene,
        &AcousticQuery {
            source: [0., 0., -4.],
            listener: [0.; 3],
            settings: AcousticSource {
                air_absorption: [0.; 3],
                ..Default::default()
            },
        },
    )?;
    let mut pcm = Vec::with_capacity((rate * 9 * 4 + 44) as usize);
    let bytes = rate * 9 * 4;
    pcm.extend(b"RIFF");
    pcm.extend((bytes + 36).to_le_bytes());
    pcm.extend(b"WAVEfmt ");
    pcm.extend(16u32.to_le_bytes());
    pcm.extend(1u16.to_le_bytes());
    pcm.extend(2u16.to_le_bytes());
    pcm.extend(rate.to_le_bytes());
    pcm.extend((rate * 4).to_le_bytes());
    pcm.extend(4u16.to_le_bytes());
    pcm.extend(16u16.to_le_bytes());
    pcm.extend(b"data");
    pcm.extend(bytes.to_le_bytes());
    for stage in 0..3 {
        mixer.set_acoustic_gains(
            1,
            voice,
            Some(if stage == 1 {
                path_result.gains
            } else {
                [1.; 3]
            }),
        )?;
        for _ in 0..300 {
            let mut block = [[0.; 2]; 480];
            mixer.render(&mut block)?;
            for s in block.into_iter().flatten() {
                pcm.extend(((s.clamp(-1., 1.) * 32767.).round() as i16).to_le_bytes());
            }
        }
    }
    std::fs::write(&path, pcm)?;
    println!(
        "{path}: 9 seconds, stereo 48 kHz; 0–3s clear, 3–6s wall, 6–9s clear; gains {:?}",
        path_result.gains
    );
    Ok(())
}
