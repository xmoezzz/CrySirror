// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Captured geometric echoes over shared BVH/material/DSP APIs. Authored data only.
use anyhow::{Result, ensure};
use crysirror_acoustics::{
    AcousticScene, AcousticSceneSpec,
    reflection_dsp::EarlyReflections,
    reflections::{ReflectionOptions, ReflectionQuery},
};
use crysirror_audio::{
    decoder,
    mixer::{Mixer, Spatial, VoiceSpec},
};
use crysirror_core::service::PcmAudio;
fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .unwrap_or_else(|| "/tmp/crysirror-echoes".into()),
    );
    std::fs::create_dir_all(&dir)?;
    let rate = 48000u32;
    let mut seed = 17u32;
    let frames = (0..480)
        .map(|i| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let v =
                ((seed >> 8) as f32 / (1u32 << 24) as f32 * 2. - 1.) * (1. - i as f32 / 480.) * 0.8;
            [v; 2]
        })
        .collect();
    let pcm = PcmAudio::new(rate, frames)?;
    for (name, absorption, blocked) in [
        ("dry", 0., false),
        ("hard-wall", 0., false),
        ("absorber", 0.99, false),
        ("blocked", 0., true),
    ] {
        let mut spec: AcousticSceneSpec = serde_json::from_value(
            serde_json::json!({"materials":[{"absorption":[absorption,absorption,absorption],"transmission":[0,0,0],"scattering":0},{"absorption":[1,1,1],"transmission":[0,0,0],"scattering":0}],"meshes":[{"id":1,"vertices":[[-50,-50,-12],[50,-50,-12],[50,50,-12],[-50,50,-12]],"triangles":[[0,1,2],[0,2,3]]}]}),
        )?;
        if blocked {
            spec.meshes.push(serde_json::from_value(serde_json::json!({"id":2,"vertices":[[-10,-10,-6],[10,-10,-6],[10,10,-6],[-10,10,-6]],"triangles":[[0,1,2],[0,2,3]],"materials":[1,1]}))?);
        }
        let scene = AcousticScene::build(spec)?;
        let query = ReflectionQuery {
            source: [0., 0., 2.],
            listener: [0.; 3],
            listener_forward: [0., 0., -1.],
            listener_up: [0., 1., 0.],
            options: ReflectionOptions {
                reference_distance_metres: 10.,
                propagation: crysirror_acoustics::AcousticSource {
                    air_absorption: [0.; 3],
                    ..Default::default()
                },
                ..Default::default()
            },
        };
        let start = std::time::Instant::now();
        let paths = crysirror_acoustics::reflections::simulate(&scene, &query)?;
        let query_time = start.elapsed().as_secs_f64();
        let count = paths.paths.len();
        let delay = paths.paths.first().map(|p| p.excess_delay_seconds);
        let mut mixer = Mixer::default();
        let h = mixer.play(
            1,
            decoder::open_pcm(pcm.clone()),
            VoiceSpec {
                spatial: Some(Spatial {
                    position: query.source,
                    min_distance: 10.,
                    max_distance: 100.,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )?;
        if name != "dry" {
            mixer.set_reflections(1, h, Some(EarlyReflections::prepare(rate, paths, 0)?))?;
        }
        let mut out = vec![[0.; 2]; rate as usize];
        let start = std::time::Instant::now();
        for block in out.chunks_mut(512) {
            mixer.render(block)?;
        }
        let mix_time = start.elapsed().as_secs_f64();
        let peak = out.iter().flatten().fold(0f32, |a, b| a.max(b.abs()));
        ensure!(peak <= 1., "demo clips");
        let tail: f64 = out[480..]
            .iter()
            .flatten()
            .map(|x| (*x as f64).powi(2))
            .sum();
        let size = (out.len() * 4) as u32;
        let mut wave = Vec::with_capacity(size as usize + 44);
        wave.extend(b"RIFF");
        wave.extend((size + 36).to_le_bytes());
        wave.extend(b"WAVEfmt ");
        wave.extend(16u32.to_le_bytes());
        wave.extend(1u16.to_le_bytes());
        wave.extend(2u16.to_le_bytes());
        wave.extend(rate.to_le_bytes());
        wave.extend((rate * 4).to_le_bytes());
        wave.extend(4u16.to_le_bytes());
        wave.extend(16u16.to_le_bytes());
        wave.extend(b"data");
        wave.extend(size.to_le_bytes());
        for sample in out.iter().flatten() {
            wave.extend(((*sample * 32767.).round() as i16).to_le_bytes());
        }
        std::fs::write(dir.join(format!("{name}.wav")), wave)?;
        println!(
            "{name}: paths={count}; excess_delay_seconds={delay:?}; query_seconds={query_time:.6}; mix_seconds={mix_time:.6}; post_source_energy={tail:.9}; peak={peak:.6}"
        );
    }
    Ok(())
}
