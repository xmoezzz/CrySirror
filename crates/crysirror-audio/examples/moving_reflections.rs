// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Offline moving echoes: worker plans, persistent mixer history, authored PCM.
use anyhow::{Result, ensure};
use crysirror_acoustics::{
    AcousticScene,
    reflection_dsp::{EarlyReflections, ReflectionPlan},
    reflections::{ReflectionQuery, simulate},
};
use crysirror_audio::{
    decoder,
    mixer::{Mixer, Spatial, VoiceSpec},
};
use crysirror_core::service::PcmAudio;
use crysirror_tasks::TaskPool;
use std::sync::Arc;

fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .unwrap_or_else(|| "/tmp/crysirror-moving-echoes".into()),
    );
    std::fs::create_dir_all(&dir)?;
    let scene = Arc::new(AcousticScene::build(serde_json::from_value(
        serde_json::json!({
        "materials":[{"absorption":[0.1,0.2,0.4],"transmission":[0,0,0],"scattering":0}],
        "meshes":[{"id":1,"vertices":[[-30,-30,-12],[30,-30,-12],[30,30,-12],[-30,30,-12]],"triangles":[[0,1,2],[0,2,3]]}]}),
    )?)?);
    let rate = 48000u32;
    let source = PcmAudio::new(
        rate,
        (0..rate * 2)
            .map(|i| {
                let t = i as f32 / rate as f32;
                let envelope = (-25. * (t * 5.).fract()).exp();
                let value = 0.5 * envelope * (t * 600. * std::f32::consts::TAU).sin();
                [value; 2]
            })
            .collect(),
    )?;
    let pool = TaskPool::new("echo-demo", 2, 8)?;
    let mut results = Vec::new();
    for moving in [false, true] {
        let mut mixer = Mixer::default();
        let h = mixer.play(
            1,
            decoder::open_pcm(source.clone()),
            VoiceSpec {
                spatial: Some(Spatial {
                    position: [-4., 0., 2.],
                    ..Default::default()
                }),
                ..Default::default()
            },
        )?;
        // Wet-only renders isolate the moving echo from the existing direct pan.
        mixer.set_acoustic_gains(1, h, Some([0.; 3]))?;
        let mut output = Vec::new();
        let mut updates = 0;
        let mut worker_seconds = 0.;
        let mut mix_seconds = 0.;
        for step in 0..60 {
            if step < 40 && (moving || step == 0) {
                let position = [
                    if moving {
                        -4. + 8. * step as f32 / 39.
                    } else {
                        -4.
                    },
                    0.,
                    2.,
                ];
                mixer.set_spatial(
                    1,
                    h,
                    Some(Spatial {
                        position,
                        ..Default::default()
                    }),
                )?;
                let query: ReflectionQuery = serde_json::from_value(serde_json::json!({
                    "source":position,"listener":[0,0,0],"options":{
                        "max_delay_seconds":0.5,"reference_distance_metres":10,
                        "propagation":{"air_absorption":[0,0,0]}}}))?;
                let captured = scene.clone();
                let start = std::time::Instant::now();
                // Explicit offline fence. Live hosts use the nonblocking engine
                // tracker; no worker wait occurs in Mixer::render.
                if step == 0 {
                    let effect = pool
                        .spawn(move |_| {
                            EarlyReflections::prepare_tracked(rate, simulate(&captured, &query)?, 0)
                        })?
                        .wait()?;
                    mixer.set_reflections(1, h, Some(effect))?;
                } else {
                    let plan = pool
                        .spawn(move |_| {
                            ReflectionPlan::prepare(rate, simulate(&captured, &query)?, 0)
                        })?
                        .wait()?;
                    mixer.publish_reflection_plan(1, h, plan, 20)?;
                }
                worker_seconds += start.elapsed().as_secs_f64();
                updates += 1;
            }
            let mut block = vec![[0.; 2]; 2400];
            let start = std::time::Instant::now();
            mixer.render(&mut block)?;
            mix_seconds += start.elapsed().as_secs_f64();
            if mixer.is_playing(1, h) {
                mixer.retire_reflection_plan(1, h)?;
            }
            output.extend(block);
        }
        let peak = output.iter().flatten().fold(0f32, |a, b| a.max(b.abs()));
        ensure!(peak > 0.01 && peak <= 1., "silent/clipped echo demo");
        let name = if moving { "moving" } else { "stationary" };
        let size = (output.len() * 4) as u32;
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
        for v in output.iter().flatten() {
            wave.extend(((*v * 32767.).round() as i16).to_le_bytes());
        }
        std::fs::write(dir.join(format!("{name}.wav")), wave)?;
        println!(
            "{name}: updates={updates}; worker_wall_seconds={worker_seconds:.6}; mix_seconds={mix_seconds:.6}; peak={peak:.6}"
        );
        results.push(output);
    }
    let difference: f64 = results[0]
        .iter()
        .flatten()
        .zip(results[1].iter().flatten())
        .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
        .sum();
    ensure!(difference > 1., "moving plans did not change PCM");
    println!("moving_vs_stationary_squared_difference={difference:.9}");
    Ok(())
}
